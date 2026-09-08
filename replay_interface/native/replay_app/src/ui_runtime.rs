
use game_ui::assets::AssetResolver;
use replay_core::NamedCountry;
use replay_core::{ArmyBattleplanOrderKey, WorldFrame};
use std::sync::{Arc, Mutex};

use crate::d3d::Gfx;
use crate::text::ImageRenderer;
use crate::Result;

#[cfg(test)]
#[path = "ui_runtime/xui08_public_entry_tests.rs"]
mod xui08_public_entry_tests;

#[cfg(test)]
#[path = "ui_runtime/xui08_recording_dispatch_tests.rs"]
mod xui08_recording_dispatch_tests;


#[derive(Clone, Copy, Default)]
pub struct ArmyBattleplanUiSupplementalInputs<'a> {
    pub selected_owner: Option<replay_core::ArmyBattleplanOwnerKey>,
    pub selected_order_key: Option<&'a ArmyBattleplanOrderKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverOccupationOutcome {
    pub surface: Option<Arc<game_ui::occupation_binding::AdmittedOccupationSurface>>,
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Xui16PanelRenderPlan<'a> {
    panel: game_ui::occupation_compositor::OccupationPanel,
    selected_country: &'a str,
    source_record_id: &'a str,
}

fn plan_xui16_panel<'a>(
    viewer: &'a game_ui::production_renderer::ObserverViewerUiState,
    unavailable_reason: Option<&str>,
    surface_source_record_id: Option<&'a str>,
) -> std::result::Result<Option<Xui16PanelRenderPlan<'a>>, String> {
    use game_ui::occupation_compositor::OccupationPanel;
    if viewer.occupation.panel == OccupationPanel::Closed {
        return Ok(None);
    }
    // A COMPARISON, and only a comparison: there is no frame here, so this
    // function asks the census about nothing. What it renders is the XUI16
    // viewer's own country, which `occupation_compositor` admits against the
    // frame; this only checks that the two agree.
    if !viewer.selected_country.is_selected() {
        return Err("XUI16 panel is open without an observer-selected country".to_string());
    }
    // THE XUI16 VIEWER HAS NO COUNTRY THIS FRAME. That is what an observer
    // selection this hour's census names nobody for leaves behind, and it is
    // not a broken viewer: there is no panel plan, and the cause travels on the
    // occupation outcome's own refusal.
    if viewer.occupation.selected_country.trim().is_empty() {
        return Ok(None);
    }
    if !viewer
        .selected_country
        .is(&viewer.occupation.selected_country)
    {
        return Err(format!(
            "XUI16 viewer country {:?} differs from the observer-selected country",
            viewer.occupation.selected_country
        ));
    }
    let selected_country = viewer.occupation.selected_country.as_str();
    if let Some(reason) = unavailable_reason {
        return Err(format!(
            "XUI16 panel is open but its exact semantic surface is unavailable: {reason}"
        ));
    }
    let source_record_id = surface_source_record_id
        .ok_or_else(|| "XUI16 panel is open without an admitted semantic surface".to_string())?;
    Ok(Some(Xui16PanelRenderPlan {
        panel: viewer.occupation.panel,
        selected_country,
        source_record_id,
    }))
}

fn execute_observer_layers<I, O, E>(
    base: &I,
    overlay: std::result::Result<Option<O>, E>,
    overlay_image: impl Fn(&O) -> &I,
    mut upload_and_draw: impl FnMut(&I) -> std::result::Result<(), E>,
) -> std::result::Result<Option<O>, E> {
    upload_and_draw(base)?;
    let overlay = overlay?;
    if let Some(output) = overlay.as_ref() {
        upload_and_draw(overlay_image(output))?;
    }
    Ok(overlay)
}

pub struct ProductObserverOccupationRuntime {
    assembler: Option<game_ui::occupation_binding::OccupationSurfaceAssembler>,
    initialization_error: Option<String>,
    last_outcome: Mutex<Option<OccupationOutcomeCacheEntry>>,
}

#[derive(Clone)]
struct OccupationOutcomeCacheEntry {
    frame: Arc<WorldFrame>,
    /// The selector this outcome was computed FOR, verbatim. A cache key is
    /// compared, never published, so it is not an identity and is not asked
    /// about -- and folding a contended selection into `None` here made one
    /// entry answer for two different selections (Codex round 51).
    /// THE RAW SELECTION AS THE KEY (lane UB, M9): a contended selection
    /// must not share a cache entry with "no selection", and the type is a key
    /// precisely because no accessor spells it.
    cache_key_country: game_ui::stored_country_selection::StoredCountrySelection,
    outcome: ObserverOccupationOutcome,
}

fn cached_occupation_outcome(
    cache: &Mutex<Option<OccupationOutcomeCacheEntry>>,
    frame: &Arc<WorldFrame>,
    selected_country: &game_ui::stored_country_selection::StoredCountrySelection,
) -> Option<ObserverOccupationOutcome> {
    cache
        .lock()
        .ok()
        .and_then(|cache| {
            cache
                .as_ref()
                .filter(|entry| {
                    Arc::ptr_eq(&entry.frame, frame)
                        && &entry.cache_key_country == selected_country
                })
                .cloned()
        })
        .map(|entry| entry.outcome)
}

impl ProductObserverOccupationRuntime {
    pub fn new(
        resolver: &map_viewer::GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        localisation_language: &str,
    ) -> Self {
        match AssetResolver::from_content_admission(resolver, admission).and_then(|assets| {
            game_ui::occupation_binding::OccupationSurfaceAssembler::new(
                &assets,
                localisation_language,
            )
        }) {
            Ok(assembler) => Self {
                assembler: Some(assembler),
                initialization_error: None,
                last_outcome: Mutex::new(None),
            },
            Err(error) => Self {
                assembler: None,
                initialization_error: Some(format!(
                    "XUI16 admitted effective-content initialization failed: {error}"
                )),
                last_outcome: Mutex::new(None),
            },
        }
    }

    pub fn refresh(
        &self,
        frame: &Arc<WorldFrame>,
        selected_country: &game_ui::stored_country_selection::StoredCountrySelection,
    ) -> ObserverOccupationOutcome {
        if let Some(outcome) =
            cached_occupation_outcome(&self.last_outcome, frame, selected_country)
        {
            return outcome;
        }
        let supplement_view = frame.occupation_ui_supplement_exact();
        let Some(assembler) = self.assembler.as_ref() else {
            let outcome = ObserverOccupationOutcome {
                surface: None,
                unavailable_reason: self.initialization_error.clone(),
            };
            self.store_cached_outcome(frame, selected_country, &outcome);
            return outcome;
        };
        if supplement_view.status != replay_core::ViewStatus::Present {
            let outcome = ObserverOccupationOutcome {
                surface: None,
                unavailable_reason: Some(format!(
                    "XUI16_D66_SUPPLEMENT_{:?}",
                    supplement_view.status
                )),
            };
            self.store_cached_outcome(frame, selected_country, &outcome);
            return outcome;
        }
        // THE SELECTION, not a name: the assembler asks this frame itself, so
        // a contended selection reaches its own refusal and keeps its cause.
        let outcome = match assembler.assemble_admitted(frame.as_ref(), selected_country) {
            Ok(surface) => ObserverOccupationOutcome {
                surface: Some(Arc::new(surface)),
                unavailable_reason: None,
            },
            Err(gap) => ObserverOccupationOutcome {
                surface: None,
                unavailable_reason: Some(gap.to_string()),
            },
        };
        self.store_cached_outcome(frame, selected_country, &outcome);
        outcome
    }

    fn store_cached_outcome(
        &self,
        frame: &Arc<WorldFrame>,
        selected_country: &game_ui::stored_country_selection::StoredCountrySelection,
        outcome: &ObserverOccupationOutcome,
    ) {
        if let Ok(mut cache) = self.last_outcome.lock() {
            *cache = Some(OccupationOutcomeCacheEntry {
                frame: Arc::clone(frame),
                cache_key_country: selected_country.clone(),
                outcome: outcome.clone(),
            });
        }
    }
}

// Clone only. The outcome carries the d79-derived semantic receipts, so `{:?}`
// of the whole value would format them into one string and `==` would turn them
// into a boolean -- either is a route from d79 to a pixel that names none of its
// fields. EIR-NE-017.
#[derive(Clone)]
pub struct ObserverUiOutcome {
    pub authored_roots: Vec<String>,
    pub authored_panel_draw_count: usize,
    pub authored_window_snapshot: Option<game_ui::window_manager_law::ReplayWindowManagerSnapshot>,
    /// The selection this outcome was produced for, carried on as STATE.
    /// Nothing renders it: a surface that names a country carries the name
    /// its own census admitted.
    pub selected_country: game_ui::stored_country_selection::StoredCountrySelection,
    pub trade_panel_open: bool,
    pub decision_panel_open: bool,
    pub trade_semantic_state: Option<game_ui::trade_state::AdmittedTradeScreen>,
    pub decision_semantic_state: Option<game_ui::production_renderer::ObserverDecisionSurface>,
    pub focus_open: bool,
    pub focus_country: Option<String>,
    pub focus_semantic_available: bool,
    pub focus_pixels_available: bool,
    pub focus_measured_empty: bool,
    pub trade_available: bool,
    pub trade_measured_empty: bool,
    pub production_lines_open: bool,
    pub production_lines_country: Option<String>,
    pub production_lines_available: bool,
    pub production_lines_applied_scroll_offset: Option<i32>,
    pub designer_open: bool,
    pub designer_country: Option<String>,
    pub designer_available: bool,
    pub designer_measured_empty: bool,
    pub characters_open: bool,
    pub characters_country: Option<String>,
    pub characters_available: bool,
    pub character_applied_scroll_row: Option<usize>,
    pub character_input_region: Option<game_ui::character_ui::CharacterInputRegion>,
    pub character_hit_targets: Vec<game_ui::character_ui::CharacterHitTarget>,
    pub character_semantic_state: Option<game_ui::character_ui::ObserverCharacterUi>,
    pub research_open: bool,
    /// The selection the research surface was produced for, carried on as
    /// STATE. Nothing renders it.
    pub research_country: game_ui::stored_country_selection::StoredCountrySelection,
    pub research_available: bool,
    pub research_folder: Option<String>,
    pub research_input_region: Option<game_ui::research_compositor::ResearchInputRegion>,
    pub research_node_hit_targets: Vec<game_ui::research_compositor::ResearchNodeHitTarget>,
    pub politics_open: bool,
    /// The country the XUI11 panel is bound to, carried on as STATE.
    pub politics_country: game_ui::stored_country_selection::StoredCountrySelection,
    pub politics_available: bool,
    pub politics_input_region: Option<game_ui::politics_state::PoliticsInputRegion>,
    pub power_balance_open: bool,
    pub power_balance_available: bool,
    pub power_balance_panel_region:
        Option<game_ui::power_balance_compositor::PowerBalanceInputRegion>,
    pub peace_conference_open: bool,
    pub peace_conference_available: bool,
    pub peace_conference_input_region:
        Option<game_ui::peace_conference_ui::PeaceConferenceInputRegion>,
    pub combat_open: bool,
    pub combat_panel: Option<game_ui::combat_detail_canvas::Xui20ViewerPanel>,
    pub combat_available: bool,
    pub combat_input_region: Option<game_ui::combat_detail_canvas::Xui20InputRegion>,
    pub semantic_receipts: Vec<String>,
    pub feature_local_gaps: Vec<String>,
    pub abilities_open: bool,
    pub abilities_country: Option<String>,
    pub abilities_available: bool,
    pub abilities_measured_empty: bool,
    pub ability_button_regions: Vec<game_ui::active_abilities::ObserverAbilityButtonRegion>,
    pub event_history_available: bool,
    pub event_pixels_available: bool,
    pub event_measured_empty: bool,
    pub event_visible_sequences: Vec<i64>,
    pub event_focused_sequence: Option<i64>,
    pub gaps: Vec<game_ui::production_renderer::ObserverUiGap>,
    pub occupation_applied_scroll_row: Option<usize>,
    pub occupation_input_region: Option<game_ui::occupation_compositor::OccupationInputRegion>,
    pub promoted_extension_packets: Vec<String>,
    /// Every localisation miss this frame's UI named: the renderer's own sink
    /// plus the occupation surface, which is composed outside the renderer.
    pub localisation_misses: replay_core::LocalisationMisses,
}

#[derive(Clone, Debug)]
pub struct ProductionLinesUiSupplementalInputs {
    producer: std::result::Result<
        Option<game_ui::production_lines::Xui12ProducerFramePayload>,
        game_ui::production_lines::Xui12CanvasLayoutRejection,
    >,
}

impl Default for ProductionLinesUiSupplementalInputs {
    fn default() -> Self {
        Self { producer: Ok(None) }
    }
}

impl ProductionLinesUiSupplementalInputs {
    pub fn from_frame(frame: &replay_core::WorldFrame) -> Self {
        Self {
            producer: game_ui::production_lines::xui12_producer_payload_from_frame(frame),
        }
    }

    fn producer(
        &self,
    ) -> std::result::Result<
        Option<&game_ui::production_lines::Xui12ProducerFramePayload>,
        game_ui::production_lines::Xui12CanvasLayoutRejection,
    > {
        self.producer
            .as_ref()
            .map(|payload| payload.as_ref())
            .map_err(Clone::clone)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObserverViewerActionDisposition {
    Applied,
    SetPlaybackSpeedStep(u8),
    SelectMapmodeButton(u8),
    OpenFocusTree,
    OpenPowerBalance,
    OpenLandTheatreSettings(u32),
    CloseLandTheatreSettings(u32),
    CloseProductionLines,
    ToggleProductionLineRow(game_ui::production_lines::Xui12RowIdentity),
    RefusedAbilityActivation {
        ability_id: String,
        caster: Option<game_ui::active_abilities::ObserverAbilityScope>,
        group_target: Option<game_ui::active_abilities::ObserverAbilityScope>,
    },
    RefusedEventOptionActivation {
        event_sequence: i64,
        definition_ordinal: usize,
    },
    RefusedDecisionActivation {
        source_record_id: String,
        decision_id: String,
    },
    RefusedDecisionCategoryProviderActivation {
        category_id: String,
        provider_id: String,
        stable_namespace: String,
    },
    RefusedGameplayMutation(game_ui::window_manager_law::PoliticsGameplayMutation),
}

pub struct ProductObserverUiRuntime {
    renderer: game_ui::production_renderer::ProductionUiRenderer,
    image: ImageRenderer,
    playback: game_ui::production_renderer::ObserverPlaybackUiProfile,
    static_research_catalog: Arc<game_ui::static_research::StaticResearchUiCatalog>,
    static_focus_content: Arc<game_ui::static_focus_defs::AdmittedStaticFocusUiContent>,
    scripted_gui_registry: Option<
        game_ui::content_interpreter::Interpreted<
            game_ui::scripted_gui_runtime::ScriptedGuiRegistry,
        >,
    >,
    authored_observer_ui: Option<crate::gui_definition_admission::AuthoredObserverUi>,
    content_admission: replay_core::ValidatedContentAdmission,
    decision_catalog: Option<game_ui::decision_replay::DecisionStaticCatalog>,
    initialization_receipts: Vec<String>,
    initialization_gaps: Vec<String>,
    execute_lifecycle: ObserverUiExecuteLifecycle,
    present_transaction: ObserverUiPresentTransactionOwner,
    xui05_controller: game_ui::production_renderer::ProductionXui05Controller,
    xui05_pending_owner: Option<(
        game_ui::window_manager_law::AuthenticatedReplayTooltipOwner,
        String,
    )>,
    xui05_pending_controller_checkpoint:
        Option<game_ui::production_renderer::ProductionXui05Controller>,
    xui23_presented: Option<Xui23PresentedFrame>,
    xui23_pending: Option<Xui23PresentedFrame>,
    xui10_presented: Option<Xui10PresentedFrame>,
    xui10_pending: Option<Xui10PresentedFrame>,
    xui10_drag_cursor: Option<[i32; 2]>,
    observer_frame_checkpoint: Option<ObserverUiFrameCheckpoint>,
    xui09_presented: Option<Xui09PresentedFrame>,
    xui09_pending: Option<Xui09PresentedFrame>,
    xui09_pending_staged: bool,
    xui09_pending_viewer_checkpoint: Option<game_ui::research_compositor::ResearchViewerState>,
    xui09_drag_cursor: Option<[i32; 2]>,
    xui09_cursor: Option<(i32, i32)>,
    xui11_presented: Option<Xui11PresentedFrame>,
    xui11_history_anchor: Option<Xui11HistoryAnchor>,
    xui11_pending: Option<Xui11PresentedFrame>,
    xui11_pending_staged: bool,
    xui11_pending_viewer_checkpoint: Option<game_ui::politics_state::PoliticsViewerState>,
    xui11_cursor: Option<(i32, i32)>,
    xui18_presented: Option<Xui18PresentedFrame>,
    xui18_pending: Option<Xui18PresentedFrame>,
    xui18_pending_staged: bool,
    xui18_pending_viewer_checkpoint:
        Option<game_ui::power_balance_compositor::PowerBalanceViewerState>,
    xui18_cursor: Option<(i32, i32)>,
    xui19_presented: Option<Xui19PresentedFrame>,
    xui19_history_anchor: Option<Xui19HistoryAnchor>,
    xui19_pending: Option<Xui19PresentedFrame>,
    xui19_pending_staged: bool,
    xui19_pending_viewer_checkpoint: Option<(bool, i32, std::collections::BTreeSet<String>)>,
    xui19_cursor: Option<(i32, i32)>,
    xui08_presented: Option<Xui08PresentedFrame>,
    xui08_history_anchor: Option<Xui08HistoryAnchor>,
    xui08_pending: Option<Xui08PresentedFrame>,
    xui08_frame_clock: game_ui::compose::ContainerFrameClock,
    xui08_pending_staged: bool,
    xui08_pending_viewer_checkpoint:
        Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
    xui08_pending_viewer_input: Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
    xui08_cursor: Option<(i32, i32)>,
    xui14_presented: Option<Xui14PresentedFrame>,
    xui14_history_anchor: Option<Xui14HistoryAnchor>,
    xui14_pending: Option<Xui14PresentedFrame>,
    xui14_pending_staged: bool,
    xui14_pending_viewer_checkpoint:
        Option<game_ui::intelligence_presentation::IntelligenceViewerState>,
    xui14_cursor: Option<(i32, i32)>,
    xui17_cursor: Option<(i32, i32)>,
    xui20_presented: Option<Xui20PresentedFrame>,
    xui20_history_anchor: Option<Xui20HistoryAnchor>,
    xui20_pending: Option<Xui20PresentedFrame>,
    xui20_pending_staged: bool,
    xui20_pending_viewer_checkpoint: Option<game_ui::combat_detail_canvas::Xui20ViewerState>,
    xui20_pending_cursor_checkpoint: Option<(i32, i32)>,
    xui20_cursor: Option<(i32, i32)>,
    xui21_presented: Option<Xui21PresentedFrame>,
    xui21_pending: Option<Xui21PresentedFrame>,
    xui21_pending_staged: bool,
    xui21_pending_viewer_checkpoint: Option<game_ui::event_chrome::EventPopupViewerState>,
    xui21_cursor: Option<(i32, i32)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ObserverUiExecuteStage {
    #[default]
    Idle,
    EarlyCandidates,
    AuthoredCandidate,
    LateCandidates,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ObserverUiExecuteLifecycle {
    stage: ObserverUiExecuteStage,
}

impl ObserverUiExecuteLifecycle {
    fn begin(&mut self) -> Result<()> {
        if self.stage != ObserverUiExecuteStage::Idle {
            return Err("observer UI previous execute has no Present disposition".into());
        }
        self.stage = ObserverUiExecuteStage::EarlyCandidates;
        Ok(())
    }

    fn mark_authored_candidate(&mut self) {
        self.stage = ObserverUiExecuteStage::AuthoredCandidate;
    }

    fn mark_late_candidates(&mut self) {
        self.stage = ObserverUiExecuteStage::LateCandidates;
    }

    fn authored_frame_staged(self) -> bool {
        matches!(
            self.stage,
            ObserverUiExecuteStage::AuthoredCandidate | ObserverUiExecuteStage::LateCandidates
        )
    }

    fn finish(&mut self) {
        self.stage = ObserverUiExecuteStage::Idle;
    }

    fn finish_ownerless_execute_error(
        &mut self,
        frame_pending: bool,
        owner_pending: bool,
    ) -> Result<bool> {
        if !frame_pending && !owner_pending {
            self.finish();
            return Ok(true);
        }
        if frame_pending != owner_pending {
            return Err("observer UI execute rollback found split Present ownership".into());
        }
        Ok(false)
    }
}

#[derive(Clone, Debug)]
struct ObserverUiFrameCheckpoint {
    authored_input: Option<crate::gui_definition_admission::AuthoredObserverUiInputCheckpoint>,
    xui05_controller: game_ui::production_renderer::ProductionXui05Controller,
    xui23_presented: Option<Xui23PresentedFrame>,
    xui10_presented: Option<Xui10PresentedFrame>,
    xui10_drag_cursor: Option<[i32; 2]>,
    xui09_presented: Option<Xui09PresentedFrame>,
    xui09_drag_cursor: Option<[i32; 2]>,
    xui09_cursor: Option<(i32, i32)>,
    xui11_presented: Option<Xui11PresentedFrame>,
    xui11_history_anchor: Option<Xui11HistoryAnchor>,
    xui11_cursor: Option<(i32, i32)>,
    xui18_presented: Option<Xui18PresentedFrame>,
    xui18_cursor: Option<(i32, i32)>,
    xui19_presented: Option<Xui19PresentedFrame>,
    xui19_history_anchor: Option<Xui19HistoryAnchor>,
    xui19_cursor: Option<(i32, i32)>,
    xui08_presented: Option<Xui08PresentedFrame>,
    xui08_history_anchor: Option<Xui08HistoryAnchor>,
    xui08_pending_viewer_input: Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
    xui08_cursor: Option<(i32, i32)>,
    xui14_presented: Option<Xui14PresentedFrame>,
    xui14_history_anchor: Option<Xui14HistoryAnchor>,
    xui14_cursor: Option<(i32, i32)>,
    xui17_cursor: Option<(i32, i32)>,
    xui20_presented: Option<Xui20PresentedFrame>,
    xui20_history_anchor: Option<Xui20HistoryAnchor>,
    xui20_cursor: Option<(i32, i32)>,
    xui21_presented: Option<Xui21PresentedFrame>,
    xui21_cursor: Option<(i32, i32)>,
}

impl ObserverUiFrameCheckpoint {
    fn capture(
        _viewer: &game_ui::production_renderer::ObserverViewerUiState,
        runtime: &ProductObserverUiRuntime,
    ) -> Result<Self> {
        Ok(Self {
            authored_input: runtime
                .authored_observer_ui
                .as_ref()
                .map(crate::gui_definition_admission::AuthoredObserverUi::input_checkpoint)
                .transpose()?,
            xui05_controller: runtime.xui05_controller.clone(),
            xui23_presented: runtime.xui23_presented.clone(),
            xui10_presented: runtime.xui10_presented.clone(),
            xui10_drag_cursor: runtime.xui10_drag_cursor,
            xui09_presented: runtime.xui09_presented.clone(),
            xui09_drag_cursor: runtime.xui09_drag_cursor,
            xui09_cursor: runtime.xui09_cursor,
            xui11_presented: runtime.xui11_presented.clone(),
            xui11_history_anchor: runtime.xui11_history_anchor.clone(),
            xui11_cursor: runtime.xui11_cursor,
            xui18_presented: runtime.xui18_presented.clone(),
            xui18_cursor: runtime.xui18_cursor,
            xui19_presented: runtime.xui19_presented.clone(),
            xui19_history_anchor: runtime.xui19_history_anchor.clone(),
            xui19_cursor: runtime.xui19_cursor,
            xui08_presented: runtime.xui08_presented.clone(),
            xui08_history_anchor: runtime.xui08_history_anchor.clone(),
            xui08_pending_viewer_input: runtime.xui08_pending_viewer_input.clone(),
            xui08_cursor: runtime.xui08_cursor,
            xui14_presented: runtime.xui14_presented.clone(),
            xui14_history_anchor: runtime.xui14_history_anchor.clone(),
            xui14_cursor: runtime.xui14_cursor,
            xui17_cursor: runtime.xui17_cursor,
            xui20_presented: runtime.xui20_presented.clone(),
            xui20_history_anchor: runtime.xui20_history_anchor.clone(),
            xui20_cursor: runtime.xui20_cursor,
            xui21_presented: runtime.xui21_presented.clone(),
            xui21_cursor: runtime.xui21_cursor,
        })
    }

    fn restore(
        self,
        _viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        runtime: &mut ProductObserverUiRuntime,
    ) -> Result<()> {
        runtime.xui05_controller = self.xui05_controller;
        runtime.xui23_presented = self.xui23_presented;
        runtime.xui10_presented = self.xui10_presented;
        runtime.xui10_drag_cursor = self.xui10_drag_cursor;
        runtime.xui09_presented = self.xui09_presented;
        runtime.xui09_drag_cursor = self.xui09_drag_cursor;
        runtime.xui09_cursor = self.xui09_cursor;
        runtime.xui11_presented = self.xui11_presented;
        runtime.xui11_history_anchor = self.xui11_history_anchor;
        runtime.xui11_cursor = self.xui11_cursor;
        runtime.xui18_presented = self.xui18_presented;
        runtime.xui18_cursor = self.xui18_cursor;
        runtime.xui19_presented = self.xui19_presented;
        runtime.xui19_history_anchor = self.xui19_history_anchor;
        runtime.xui19_cursor = self.xui19_cursor;
        runtime.xui08_presented = self.xui08_presented;
        runtime.xui08_history_anchor = self.xui08_history_anchor;
        runtime.xui08_pending_viewer_input = self.xui08_pending_viewer_input;
        runtime.xui08_cursor = self.xui08_cursor;
        runtime.xui14_presented = self.xui14_presented;
        runtime.xui14_history_anchor = self.xui14_history_anchor;
        runtime.xui14_cursor = self.xui14_cursor;
        runtime.xui17_cursor = self.xui17_cursor;
        runtime.xui20_presented = self.xui20_presented;
        runtime.xui20_history_anchor = self.xui20_history_anchor;
        runtime.xui20_cursor = self.xui20_cursor;
        runtime.xui21_presented = self.xui21_presented;
        runtime.xui21_cursor = self.xui21_cursor;
        match (runtime.authored_observer_ui.as_mut(), self.authored_input) {
            (Some(authored), Some(checkpoint)) => authored.restore_input_checkpoint(checkpoint),
            (None, None) => Ok(()),
            _ => Err("authored observer UI lifetime changed inside Present transaction".into()),
        }
    }

    fn make_runtime_invalidation_durable(
        &mut self,
        _viewer: &game_ui::production_renderer::ObserverViewerUiState,
        runtime: &ProductObserverUiRuntime,
    ) -> Result<()> {
        self.authored_input = runtime
            .authored_observer_ui
            .as_ref()
            .map(crate::gui_definition_admission::AuthoredObserverUi::input_checkpoint)
            .transpose()?;
        self.xui05_controller = runtime.xui05_controller.clone();
        self.xui23_presented = runtime.xui23_presented.clone();
        self.xui10_presented = runtime.xui10_presented.clone();
        self.xui10_drag_cursor = runtime.xui10_drag_cursor;
        self.xui09_presented = runtime.xui09_presented.clone();
        self.xui09_drag_cursor = runtime.xui09_drag_cursor;
        self.xui09_cursor = runtime.xui09_cursor;
        self.xui11_presented = runtime.xui11_presented.clone();
        self.xui11_history_anchor = runtime.xui11_history_anchor.clone();
        self.xui11_cursor = runtime.xui11_cursor;
        self.xui18_presented = runtime.xui18_presented.clone();
        self.xui18_cursor = runtime.xui18_cursor;
        self.xui19_presented = runtime.xui19_presented.clone();
        self.xui19_history_anchor = runtime.xui19_history_anchor.clone();
        self.xui19_cursor = runtime.xui19_cursor;
        self.xui08_presented = runtime.xui08_presented.clone();
        self.xui08_history_anchor = runtime.xui08_history_anchor.clone();
        self.xui08_pending_viewer_input = runtime.xui08_pending_viewer_input.clone();
        self.xui08_cursor = runtime.xui08_cursor;
        self.xui14_presented = runtime.xui14_presented.clone();
        self.xui14_history_anchor = runtime.xui14_history_anchor.clone();
        self.xui14_cursor = runtime.xui14_cursor;
        self.xui17_cursor = runtime.xui17_cursor;
        self.xui20_presented = runtime.xui20_presented.clone();
        self.xui20_history_anchor = runtime.xui20_history_anchor.clone();
        self.xui20_cursor = runtime.xui20_cursor;
        self.xui21_presented = runtime.xui21_presented.clone();
        self.xui21_cursor = runtime.xui21_cursor;
        Ok(())
    }

    fn commit(self) {}
}

#[derive(Clone, Debug)]
struct ObserverViewerPresentTransaction {
    checkpoint: game_ui::production_renderer::ObserverViewerUiState,
}

impl ObserverViewerPresentTransaction {
    fn begin(viewer: &game_ui::production_renderer::ObserverViewerUiState) -> Self {
        Self {
            checkpoint: viewer.clone(),
        }
    }

    fn reject(self, viewer: &mut game_ui::production_renderer::ObserverViewerUiState) {
        *viewer = self.checkpoint;
    }

    fn make_politics_frame_invalidation_durable(
        &mut self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
    ) {
        self.checkpoint.politics.displayed_scroll_fixed = viewer.politics.displayed_scroll_fixed;
        self.checkpoint.politics.target_scroll_fixed = viewer.politics.target_scroll_fixed;
        self.checkpoint.politics.selected_idea_id = viewer.politics.selected_idea_id.clone();
    }

    fn make_resize_pointer_release_durable(
        &mut self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
    ) {
        self.checkpoint.trade.active_scrollbar_drag = viewer.trade.active_scrollbar_drag.clone();
        // Rejection cannot resurrect the discarded viewport instance. Reset
        // only geometry/capture in the checkpoint, preserving prior intent.
        self.checkpoint.peace_conference.invalidate_viewport_container();
    }

    fn commit(self) {}
}

fn release_xui10_pointer_capture_durably(
    drag_cursor: &mut Option<[i32; 2]>,
    frame_checkpoint: &mut Option<ObserverUiFrameCheckpoint>,
) -> bool {
    let released = drag_cursor.take().is_some();
    if let Some(checkpoint) = frame_checkpoint.as_mut() {
        checkpoint.xui10_drag_cursor = None;
    }
    released
}

#[derive(Clone, Debug)]
struct Xui21PresentedPopup {
    event_sequence: i64,
    root_source: String,
    root_window: String,
    root_region: game_ui::event_chrome::EventPopupInputRegion,
    minimize_region: game_ui::event_chrome::EventPopupInputRegion,
    minimize_child_path: Vec<u32>,
    minimize_tooltip_key: Option<String>,
    option_targets: Vec<game_ui::event_chrome::EventOptionHitTarget>,
}

#[derive(Clone, Debug)]
struct Xui21PresentedFrame {
    country_tag: String,
    visible_sequences: Vec<i64>,
    focused_sequence: Option<i64>,
    popups: Vec<Xui21PresentedPopup>,
}

fn xui21_presented_frame_is_interactive(frame: &Xui21PresentedFrame) -> bool {
    let Some(focused_sequence) = frame.focused_sequence else {
        return false;
    };
    let popup_sequences = frame
        .popups
        .iter()
        .map(|popup| popup.event_sequence)
        .collect::<std::collections::BTreeSet<_>>();
    let visible_sequences = frame
        .visible_sequences
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    !frame.popups.is_empty()
        && frame.popups.len() == frame.visible_sequences.len()
        && popup_sequences.len() == frame.popups.len()
        && visible_sequences.len() == frame.visible_sequences.len()
        && popup_sequences == visible_sequences
        && frame.visible_sequences.contains(&focused_sequence)
        && frame
            .popups
            .last()
            .is_some_and(|popup| popup.event_sequence == focused_sequence)
}

fn xui21_staged_candidate(
    country_tag: Option<&String>,
    event_tick: Option<i64>,
    event_history_available: bool,
    event_pixels_available: bool,
    visible_sequences: &[i64],
    focused_sequence: Option<i64>,
    popups: Vec<Xui21PresentedPopup>,
) -> Option<Xui21PresentedFrame> {
    let frame = Xui21PresentedFrame {
        country_tag: country_tag?.clone(),
        visible_sequences: visible_sequences.to_vec(),
        focused_sequence,
        popups,
    };
    (event_tick.is_some()
        && event_history_available
        && event_pixels_available
        && xui21_presented_frame_is_interactive(&frame))
    .then_some(frame)
}

fn xui21_matching_presented_frame<'a>(
    selected_country: &game_ui::stored_country_selection::StoredCountrySelection,
    presented: Option<&'a Xui21PresentedFrame>,
) -> Option<&'a Xui21PresentedFrame> {
    presented
        .filter(|frame| selected_country.is(frame.country_tag.as_str()))
        .filter(|frame| xui21_presented_frame_is_interactive(frame))
}

fn begin_xui21_staged_candidate(
    viewer: &game_ui::event_chrome::EventPopupViewerState,
    viewer_checkpoint: &mut Option<game_ui::event_chrome::EventPopupViewerState>,
    pending: &Option<Xui21PresentedFrame>,
    pending_staged: bool,
) -> Result<()> {
    if viewer_checkpoint.is_some() || pending.is_some() || pending_staged {
        return Err("XUI21 staged state was not resolved before execute".into());
    }
    *viewer_checkpoint = Some(viewer.clone());
    Ok(())
}

fn stage_xui21_candidate(
    viewer_checkpoint: &Option<game_ui::event_chrome::EventPopupViewerState>,
    pending: &mut Option<Xui21PresentedFrame>,
    pending_staged: &mut bool,
    candidate: Option<Xui21PresentedFrame>,
) -> Result<()> {
    if viewer_checkpoint.is_none() || pending.is_some() || *pending_staged {
        return Err("XUI21 candidate cannot be staged without one fresh viewer checkpoint".into());
    }
    *pending = candidate;
    *pending_staged = true;
    Ok(())
}

fn commit_xui21_staged_candidate(
    viewer_checkpoint: &mut Option<game_ui::event_chrome::EventPopupViewerState>,
    pending: &mut Option<Xui21PresentedFrame>,
    pending_staged: &mut bool,
) -> Result<Option<Xui21PresentedFrame>> {
    if !*pending_staged {
        return Ok(None);
    }
    viewer_checkpoint
        .take()
        .ok_or("XUI21 viewer checkpoint is absent for presented frame")?;
    *pending_staged = false;
    Ok(pending.take())
}

fn rollback_xui21_staged_candidate(
    viewer: &mut game_ui::event_chrome::EventPopupViewerState,
    viewer_checkpoint: &mut Option<game_ui::event_chrome::EventPopupViewerState>,
    pending: &mut Option<Xui21PresentedFrame>,
    pending_staged: &mut bool,
) -> Result<()> {
    if let Some(checkpoint) = viewer_checkpoint.take() {
        *viewer = checkpoint;
        *pending = None;
        *pending_staged = false;
        return Ok(());
    }
    if *pending_staged || pending.is_some() {
        return Err("XUI21 staged state exists without a viewer checkpoint".into());
    }
    Ok(())
}

fn cycle_xui21_presented_focus(
    presented: &Xui21PresentedFrame,
    viewer: &mut game_ui::event_chrome::EventPopupViewerState,
    backwards: bool,
) -> Result<()> {
    if !xui21_presented_frame_is_interactive(presented) {
        return Err("XUI21 Presented event frame has no authenticated pixel stack".into());
    }
    if backwards {
        let current = viewer
            .focused_event_sequence
            .and_then(|focused| {
                presented
                    .visible_sequences
                    .iter()
                    .position(|value| *value == focused)
            })
            .unwrap_or(0);
        let next = if current == 0 {
            presented.visible_sequences.len() - 1
        } else {
            current - 1
        };
        viewer.focused_event_sequence = Some(presented.visible_sequences[next]);
    } else {
        viewer.cycle_focus(&presented.visible_sequences);
    }
    Ok(())
}

fn dismiss_xui21_presented_focus(
    presented: &Xui21PresentedFrame,
    viewer: &mut game_ui::event_chrome::EventPopupViewerState,
) -> Result<()> {
    if !xui21_presented_frame_is_interactive(presented) {
        return Err("XUI21 Presented event frame has no authenticated pixel stack".into());
    }
    let focused = viewer
        .focused_event_sequence
        .or(presented.focused_sequence)
        .ok_or("XUI21 Presented event stack has no focus")?;
    if !presented.visible_sequences.contains(&focused) {
        return Err("XUI21 focused event is stale for the Presented frame".into());
    }
    viewer.focused_event_sequence = Some(focused);
    viewer.dismiss_focused();
    Ok(())
}

fn press_xui21_presented_popup(
    presented: &Xui21PresentedFrame,
    viewer: &mut game_ui::event_chrome::EventPopupViewerState,
    x: i32,
    y: i32,
) -> Result<ObserverViewerActionDisposition> {
    if !xui21_presented_frame_is_interactive(presented) {
        return Err("XUI21 Presented event frame has no authenticated pixel stack".into());
    }
    let popup = presented
        .popups
        .iter()
        .rev()
        .find(|popup| popup.root_region.contains(x, y))
        .ok_or("XUI21 pointer is outside the Presented event stack")?;
    if !viewer.focus(popup.event_sequence, &presented.visible_sequences) {
        return Err("XUI21 Presented popup is absent from the visible sequence set".into());
    }
    if popup.minimize_region.contains(x, y) {
        viewer.dismiss_focused();
        return Ok(ObserverViewerActionDisposition::Applied);
    }
    if let Some(target) = popup
        .option_targets
        .iter()
        .find(|target| target.region.contains(x, y))
    {
        return Ok(
            ObserverViewerActionDisposition::RefusedEventOptionActivation {
                event_sequence: target.event_sequence,
                definition_ordinal: target.definition_ordinal,
            },
        );
    }
    Ok(ObserverViewerActionDisposition::Applied)
}

#[derive(Clone, Debug)]
struct Xui10PresentedFrame {
    country_tag: String,
    frame_hour: u64,
    input_region: game_ui::focus_observer::ObserverFocusInputRegion,
    node_regions: Vec<game_ui::focus_observer::ObserverFocusNodeRegion>,
    progress: Option<game_ui::focus_observer::ObserverFocusProgressBinding>,
}

#[derive(Clone, Debug)]
struct Xui09PresentedFrame {
    country_tag: String,
    frame_hour: u64,
    available_folders: Vec<String>,
    input_region: Option<game_ui::research_compositor::ResearchInputRegion>,
    node_hit_targets: Vec<game_ui::research_compositor::ResearchNodeHitTarget>,
    model: game_ui::research_observer::ObserverResearchModel,
}

fn xui09_inspection_target<'a>(
    targets: &'a [game_ui::research_compositor::ResearchNodeHitTarget],
    selected_technology_id: Option<&str>,
    cursor: Option<(i32, i32)>,
) -> Option<&'a game_ui::research_compositor::ResearchNodeHitTarget> {
    selected_technology_id
        .and_then(|selected| {
            targets
                .iter()
                .find(|target| target.technology_id == selected)
        })
        .or_else(|| {
            cursor.and_then(|(x, y)| targets.iter().rev().find(|target| target.contains(x, y)))
        })
}

fn xui09_matching_presented_surface<'a>(
    panel_open: bool,
    selected_country: &game_ui::stored_country_selection::StoredCountrySelection,
    presented: Option<&'a Xui09PresentedFrame>,
) -> Option<(
    &'a Xui09PresentedFrame,
    game_ui::research_compositor::ResearchInputRegion,
)> {
    panel_open
        .then_some(())
        .and_then(|_| presented)
        .filter(|frame| selected_country.is(frame.country_tag.as_str()))
        .and_then(|frame| frame.input_region.map(|region| (frame, region)))
}

fn xui09_blocks_lower_pointer_input_for_state(
    panel_open: bool,
    selected_country: &game_ui::stored_country_selection::StoredCountrySelection,
    presented: Option<&Xui09PresentedFrame>,
    x: i32,
    y: i32,
) -> bool {
    if !panel_open {
        return false;
    }
    match xui09_matching_presented_surface(panel_open, selected_country, presented) {
        Some((_, region)) => region.contains(x, y),
        None => true,
    }
}

fn close_xui09_after_composition_failure(
    viewer: &mut game_ui::research_compositor::ResearchViewerState,
    drag_cursor: &mut Option<[i32; 2]>,
    cursor: &mut Option<(i32, i32)>,
) {
    viewer.close();
    *drag_cursor = None;
    *cursor = None;
}

fn reset_xui09_country_bound_state(
    viewer: &mut game_ui::research_compositor::ResearchViewerState,
    presented: &mut Option<Xui09PresentedFrame>,
    pending: &mut Option<Xui09PresentedFrame>,
    pending_staged: &mut bool,
    pending_viewer_checkpoint: &mut Option<game_ui::research_compositor::ResearchViewerState>,
    drag_cursor: &mut Option<[i32; 2]>,
    cursor: &mut Option<(i32, i32)>,
) {
    viewer.reset_for_seek_or_country_change();
    *presented = None;
    *pending = None;
    *pending_staged = false;
    *pending_viewer_checkpoint = None;
    *drag_cursor = None;
    *cursor = None;
}

#[derive(Clone, Debug)]
struct Xui11PresentedFrame {
    country_tag: String,
    frame_hour: u64,
    capture_sequence: u64,
    record_sha256: String,
    input_region: Option<game_ui::politics_state::PoliticsInputRegion>,
    presentation: Option<game_ui::politics_state::PoliticsCountryPresentation>,
    viewport: [usize; 2],
    tooltip_targets: Vec<game_ui::politics_state::PoliticsTooltipHitTarget>,
}

#[derive(Clone, Debug)]
struct Xui18PresentedFrame {
    country_tag: String,
    frame_hour: u64,
    state_semantic_receipt_sha256: String,
    availability: Xui18PresentedAvailability,
    panel_region: Option<game_ui::power_balance_compositor::PowerBalanceInputRegion>,
    close_region: Option<game_ui::power_balance_compositor::PowerBalanceInputRegion>,
    tooltip_targets: Vec<game_ui::power_balance_compositor::PowerBalanceTooltipHitTarget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Xui18PresentedAvailability {
    Visible,
    MeasuredEmpty,
}

impl Xui18PresentedFrame {
    fn has_active_balance(&self) -> bool {
        self.availability == Xui18PresentedAvailability::Visible
    }
}

fn xui18_matching_presented_region(
    panel_open: bool,
    selected_country: &game_ui::stored_country_selection::StoredCountrySelection,
    presented: Option<&Xui18PresentedFrame>,
) -> Option<game_ui::power_balance_compositor::PowerBalanceInputRegion> {
    panel_open
        .then_some(())
        .and_then(|_| presented)
        .filter(|frame| {
            frame.has_active_balance() && selected_country.is(frame.country_tag.as_str())
        })
        .and_then(|frame| frame.panel_region)
}

fn xui18_blocks_lower_pointer_input_for_state(
    panel_open: bool,
    selected_country: &game_ui::stored_country_selection::StoredCountrySelection,
    presented: Option<&Xui18PresentedFrame>,
    x: i32,
    y: i32,
) -> bool {
    if !panel_open {
        return false;
    }
    match xui18_matching_presented_region(panel_open, selected_country, presented) {
        Some(region) => region.contains(x, y),
        None => true,
    }
}

#[derive(Clone, Debug)]
struct Xui19PresentedFrame {
    country_tag: String,
    frame_hour: i64,
    source_sequence: u64,
    previous_record_sha256: Option<String>,
    record_sha256: String,
    content_manifest_sha256: String,
    panel_drawn: bool,
    panel_rect: Option<[i32; 4]>,
    decision_grid_rect: Option<[i32; 4]>,
    max_scroll_px: i32,
    category_extents: Vec<game_ui::production_renderer::ObserverDecisionCategoryExtent>,
    card_hit_targets: Vec<game_ui::production_renderer::Xui19CardHitTarget>,
    custom_icon_hit_targets: Vec<game_ui::production_renderer::Xui19CustomIconHitTarget>,
    /// Exact inserted-child ownership retained transactionally through Present.
    /// refine descendant hit regions without changing parent ownership.
    provider_receipts: Vec<game_ui::production_renderer::ObserverDecisionCategoryProviderReceipt>,
    xui22_widget_hit_targets: Vec<game_ui::production_renderer::Xui22WidgetHitTarget>,
}

#[derive(Clone, Debug)]
struct Xui19HistoryAnchor {
    frame_hour: i64,
    source_sequence: u64,
    record_sha256: String,
}

impl Xui19HistoryAnchor {
    fn accepts(&self, candidate: &Xui19PresentedFrame) -> bool {
        if candidate.frame_hour == self.frame_hour {
            return candidate.source_sequence == self.source_sequence
                && candidate.record_sha256 == self.record_sha256;
        }
        candidate.frame_hour == self.frame_hour.saturating_add(1)
            && candidate.source_sequence > self.source_sequence
            && candidate.previous_record_sha256.as_deref() == Some(self.record_sha256.as_str())
    }
}

impl From<&Xui19PresentedFrame> for Xui19HistoryAnchor {
    fn from(frame: &Xui19PresentedFrame) -> Self {
        Self {
            frame_hour: frame.frame_hour,
            source_sequence: frame.source_sequence,
            record_sha256: frame.record_sha256.clone(),
        }
    }
}

fn xui19_rect_contains(rect: [i32; 4], x: i32, y: i32) -> bool {
    rect[2] > 0
        && rect[3] > 0
        && x >= rect[0]
        && y >= rect[1]
        && x < rect[0].saturating_add(rect[2])
        && y < rect[1].saturating_add(rect[3])
}

fn xui19_rect_intersection(left: [i32; 4], right: [i32; 4]) -> Option<[i32; 4]> {
    let x0 = left[0].max(right[0]);
    let y0 = left[1].max(right[1]);
    let x1 = left[0]
        .saturating_add(left[2])
        .min(right[0].saturating_add(right[2]));
    let y1 = left[1]
        .saturating_add(left[3])
        .min(right[1].saturating_add(right[3]));
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

fn xui19_provider_screen_rect(
    frame: &Xui19PresentedFrame,
    receipt: &game_ui::production_renderer::ObserverDecisionCategoryProviderReceipt,
) -> Option<[i32; 4]> {
    let grid = frame.decision_grid_rect?;
    let local = receipt.final_local_rect;
    if local[2] <= 0 || local[3] <= 0 {
        return None;
    }
    xui19_rect_intersection(
        [
            grid[0].saturating_add(local[0]),
            grid[1].saturating_add(local[1]),
            local[2],
            local[3],
        ],
        grid,
    )
}

fn xui19_matching_presented<'a>(
    viewer: &game_ui::production_renderer::ObserverViewerUiState,
    presented: Option<&'a Xui19PresentedFrame>,
) -> Option<&'a Xui19PresentedFrame> {
    presented.filter(|frame| viewer.selected_country.is(frame.country_tag.as_str()))
}

/// The XUI11 passive candidate, out of `execute` so the census question it has
/// to ask is a function a test can call.
///
/// Codex round 51 demonstrated the false green that put it here: the
/// stored-selection scan asks whether the FUNCTION consults the census, and
/// `execute` is large enough that deleting this one ask left the scan green
/// while `viewer.selected_country` went straight into `country_tag` -- which is
/// the round-50 defect, exactly. A 40-line function with one selector read
/// cannot hide an unasked one, and
/// `a_contended_selection_yields_no_xui11_candidate` is what this ask dies
/// without.
fn xui11_passive_candidate(
    frame: &WorldFrame,
    viewer: &game_ui::production_renderer::ObserverViewerUiState,
    history_anchor: Option<&Xui11HistoryAnchor>,
    viewport: [usize; 2],
    feature_local_gaps: &mut Vec<String>,
) -> Option<Xui11PresentedFrame> {
    let presentation = frame.politics_presentation_exact();
    let carrier = presentation.presentation.as_ref()?;
    let continuity_ok = history_anchor.is_none_or(|prior| {
        prior.accepts(
            carrier.frame_hour,
            carrier.capture_sequence,
            carrier.previous_record_sha256.as_deref(),
            &carrier.record_sha256,
        )
    });
    if !continuity_ok {
        feature_local_gaps.push(
            "XUI11:field=history:domain=_r2_politics_presentation_exact:unavailable=NON_CONTIGUOUS_OR_REWRITTEN_PASSIVE_RECORD"
                .into(),
        );
        return None;
    }
    // THE SELECTOR IS APP STATE HERE, not a parameter, and it outlives the
    // frame that vouched for it: a viewer whose selection is a key two ids
    // reach must not be handed an XUI11 candidate naming it, and the
    // closed-panel branch publishes that name into a semantic receipt and marks
    // the surface available (Codex round 50).
    let Some(selected) = viewer.selected_country.named_in(frame) else {
        feature_local_gaps.push(
            "XUI11:field=country_tag:domain=contention_census:unavailable=SELECTED_COUNTRY_IS_A_CONTENDED_KEY"
                .into(),
        );
        return None;
    };
    Some(Xui11PresentedFrame {
        country_tag: selected.tag().to_owned(),
        frame_hour: carrier.frame_hour,
        capture_sequence: carrier.capture_sequence,
        record_sha256: carrier.record_sha256.clone(),
        input_region: None,
        presentation: None,
        viewport,
        tooltip_targets: Vec::new(),
    })
}

/// The XUI19 passive candidate, and the census question it has to ask.
///
/// The ask lives HERE rather than at the call site because the panel closes
/// either way and a closed panel that says nothing is indistinguishable from no
/// selection at all: Codex round 53 deleted the caller's gap and every gate
/// stayed green. `a_contended_xui19_selection_yields_no_candidate_and_names_why`
/// is what this ask and this gap die without.
fn xui19_passive_candidate(
    frame: &WorldFrame,
    selected_country: &game_ui::stored_country_selection::StoredCountrySelection,
    contention_gap: &mut Option<String>,
) -> Option<Xui19PresentedFrame> {
    let Some(named) = selected_country.named_in(frame) else {
        // A selection this hour's census refuses is a GAP; nothing selected at
        // all is not, and the two are told apart by the refusal, not by a
        // spelling this frame does not have.
        if selected_country.refusal_in(frame)
            == Some(game_ui::stored_country_selection::StoredSelectionRefusal::ContendedKey)
        {
            *contention_gap = Some(format!(
                "XUI19:field=selected_country:domain=contention_census:unavailable={}",
                selected_country
                    .refusal_detail_in(frame)
                    .unwrap_or_default()
            ));
        }
        return None;
    };
    let selected_country = named;
    let view = frame.decision_evaluation_exact();
    let carrier = view.carrier.as_ref()?;
    view.country(selected_country.tag())?;
    Some(Xui19PresentedFrame {
        country_tag: selected_country.to_string(),
        frame_hour: carrier.frame_hour,
        source_sequence: carrier.source_sequence,
        previous_record_sha256: carrier.previous_record_sha256.clone(),
        record_sha256: carrier.record_sha256.clone(),
        content_manifest_sha256: carrier.content_manifest_sha256.clone(),
        panel_drawn: false,
        panel_rect: None,
        decision_grid_rect: None,
        max_scroll_px: 0,
        category_extents: Vec::new(),
        card_hit_targets: Vec::new(),
        custom_icon_hit_targets: Vec::new(),
        provider_receipts: Vec::new(),
        xui22_widget_hit_targets: Vec::new(),
    })
}

#[derive(Clone, Debug)]
struct Xui08PresentedFrame {
    frame_hour: u64,
    capture_sequence: u64,
    record_sha256: String,
    admitted: game_ui::readonly_design_peace::AdmittedPeaceConference,
    input_region: Option<game_ui::peace_conference_ui::PeaceConferenceInputRegion>,
    hit_targets: Vec<game_ui::peace_conference_ui::PeaceConferenceHitTarget>,
    tooltip_targets: Vec<game_ui::peace_conference_ui::PeaceConferenceTooltipHitTarget>,
}

struct Xui08InstalledSurface {
    input_region: Option<game_ui::peace_conference_ui::PeaceConferenceInputRegion>,
    visible: bool,
    receipt: String,
}

#[derive(Debug)]
enum Xui08SurfaceStageError {
    Renderer(game_ui::production_renderer::ProductionUiRejection),
    Execution(game_ui::GameUiError),
}

/// The caller's Present transaction owns both mutable arguments. Rendering and
/// blending must succeed before either candidate state or hit authority changes.
fn compose_and_install_xui08_surface(
    renderer: &game_ui::production_renderer::ProductionUiRenderer,
    world: &WorldFrame,
    viewport: [usize; 2],
    delta: f32,
    viewer: &mut game_ui::peace_conference_ui::PeaceConferenceViewerState,
    pending: &mut Xui08PresentedFrame,
    framebuffer: &mut game_ui::dds::RgbaImage,
) -> std::result::Result<Xui08InstalledSurface, Xui08SurfaceStageError> {
    if [framebuffer.width, framebuffer.height] != viewport
        || framebuffer.width.checked_mul(framebuffer.height)
            .and_then(|pixels| pixels.checked_mul(4)) != Some(framebuffer.pixels.len())
    {
        return Err(Xui08SurfaceStageError::Execution(game_ui::GameUiError::missing(
            "XUI08 destination dimensions or RGBA storage differ from the current viewport",
        )));
    }
    let (composed, mut candidate) = renderer.compose_xui08_peace_conference_surface(
        world, viewer, viewport[0], viewport[1], delta, framebuffer,
    ).map_err(Xui08SurfaceStageError::Renderer)?;
    let Some(output) = composed else {
        *viewer = candidate;
        pending.input_region = None;
        pending.hit_targets.clear();
        pending.tooltip_targets.clear();
        return Ok(Xui08InstalledSurface {
            input_region: None,
            visible: false,
            receipt: "XUI08:local-container-hidden:no-render-or-hit-authority".into(),
        });
    };
    for region in [
        game_ui::peace_conference_ui::PeaceConferenceScrollRegion::AvailableActions,
        game_ui::peace_conference_ui::PeaceConferenceScrollRegion::CurrentAndHistory,
        game_ui::peace_conference_ui::PeaceConferenceScrollRegion::ContestedPopup,
    ] {
        candidate.clamp_scroll(region, output.input_region.maximum(region))
            .map_err(Xui08SurfaceStageError::Execution)?;
    }
    // The renderer staged native blending over this framebuffer. Blending the
    // result as a transparent overlay would apply source alpha a second time.
    *framebuffer = output.image;
    let receipt = format!(
        "XUI08:record={}:participants={}:allocations={}:results={}:manager-overlay-composited",
        output.record_sha256,
        output.materialized_participant_ids.len(),
        output.materialized_allocation_ids.len(),
        output.materialized_result_ids.len(),
    );
    *viewer = candidate;
    pending.input_region = Some(output.input_region);
    pending.hit_targets = output.hit_targets;
    pending.tooltip_targets = output.tooltip_targets;
    Ok(Xui08InstalledSurface {
        input_region: pending.input_region.clone(),
        visible: true,
        receipt,
    })
}

#[derive(Clone, Debug)]
struct Xui14PresentedFrame {
    frame_hour: u64,
    capture_sequence: u64,
    record_sha256: String,
    country_tags: Vec<String>,
    input_region: Option<game_ui::intelligence_presentation::IntelligenceInputRegion>,
    tooltip_targets: Vec<game_ui::intelligence_presentation::IntelligenceTooltipHitTarget>,
}

#[derive(Clone, Debug)]
struct Xui20PresentedFrame {
    frame_hour: u64,
    capture_sequence: u64,
    record_sha256: String,
    available_panels: Vec<Xui20AvailablePanel>,
    input_region: Option<game_ui::combat_detail_canvas::Xui20InputRegion>,
    tooltip_targets: Vec<game_ui::combat_detail_canvas::Xui20TooltipHitTarget>,
}

#[derive(Clone, Debug)]
struct Xui20AvailablePanel {
    panel: game_ui::combat_detail_canvas::Xui20ViewerPanel,
    primary_scroll_max_px: i32,
    secondary_scroll_max_px: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Xui11HistoryAnchor {
    frame_hour: u64,
    capture_sequence: u64,
    record_sha256: String,
}

impl Xui11HistoryAnchor {
    fn accepts(
        &self,
        frame_hour: u64,
        capture_sequence: u64,
        previous_record_sha256: Option<&str>,
        record_sha256: &str,
    ) -> bool {
        if frame_hour == self.frame_hour {
            capture_sequence == self.capture_sequence && record_sha256 == self.record_sha256
        } else {
            frame_hour == self.frame_hour.saturating_add(1)
                && capture_sequence == self.capture_sequence.saturating_add(1)
                && previous_record_sha256 == Some(self.record_sha256.as_str())
        }
    }
}

impl From<&Xui11PresentedFrame> for Xui11HistoryAnchor {
    fn from(frame: &Xui11PresentedFrame) -> Self {
        Self {
            frame_hour: frame.frame_hour,
            capture_sequence: frame.capture_sequence,
            record_sha256: frame.record_sha256.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Xui08HistoryAnchor {
    frame_hour: u64,
    capture_sequence: u64,
    record_sha256: String,
}

impl Xui08HistoryAnchor {
    fn accepts(
        &self,
        frame_hour: u64,
        capture_sequence: u64,
        previous_record_sha256: Option<&str>,
        record_sha256: &str,
    ) -> bool {
        if frame_hour == self.frame_hour {
            capture_sequence == self.capture_sequence && record_sha256 == self.record_sha256
        } else {
            frame_hour == self.frame_hour.saturating_add(1)
                && capture_sequence == self.capture_sequence.saturating_add(1)
                && previous_record_sha256 == Some(self.record_sha256.as_str())
        }
    }
}

impl From<&Xui08PresentedFrame> for Xui08HistoryAnchor {
    fn from(frame: &Xui08PresentedFrame) -> Self {
        Self {
            frame_hour: frame.frame_hour,
            capture_sequence: frame.capture_sequence,
            record_sha256: frame.record_sha256.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Xui14HistoryAnchor {
    frame_hour: u64,
    capture_sequence: u64,
    record_sha256: String,
}

impl Xui14HistoryAnchor {
    fn accepts(
        &self,
        frame_hour: u64,
        capture_sequence: u64,
        previous_record_sha256: Option<&str>,
        record_sha256: &str,
    ) -> bool {
        if frame_hour == self.frame_hour {
            capture_sequence == self.capture_sequence && record_sha256 == self.record_sha256
        } else {
            frame_hour == self.frame_hour.saturating_add(1)
                && capture_sequence == self.capture_sequence.saturating_add(1)
                && previous_record_sha256 == Some(self.record_sha256.as_str())
        }
    }
}

impl From<&Xui14PresentedFrame> for Xui14HistoryAnchor {
    fn from(frame: &Xui14PresentedFrame) -> Self {
        Self {
            frame_hour: frame.frame_hour,
            capture_sequence: frame.capture_sequence,
            record_sha256: frame.record_sha256.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Xui20HistoryAnchor {
    frame_hour: u64,
    capture_sequence: u64,
    record_sha256: String,
}

impl Xui20HistoryAnchor {
    fn accepts(
        &self,
        frame_hour: u64,
        capture_sequence: u64,
        previous_record_sha256: Option<&str>,
        record_sha256: &str,
    ) -> bool {
        if frame_hour == self.frame_hour {
            capture_sequence == self.capture_sequence && record_sha256 == self.record_sha256
        } else {
            frame_hour == self.frame_hour.saturating_add(1)
                && capture_sequence == self.capture_sequence.saturating_add(1)
                && previous_record_sha256 == Some(self.record_sha256.as_str())
        }
    }
}

impl From<&Xui20PresentedFrame> for Xui20HistoryAnchor {
    fn from(frame: &Xui20PresentedFrame) -> Self {
        Self {
            frame_hour: frame.frame_hour,
            capture_sequence: frame.capture_sequence,
            record_sha256: frame.record_sha256.clone(),
        }
    }
}

fn xui20_available_panels(
    carrier: &replay_core::CombatPresentationExact,
) -> Result<Vec<Xui20AvailablePanel>> {
    use game_ui::combat_detail_canvas::{Xui20CombatLogTab, Xui20ViewerPanel};
    let mut panels = Vec::new();
    for presentation in &carrier.panels {
        let available = |panel| Xui20AvailablePanel {
            panel,
            primary_scroll_max_px: presentation.primary_scroll_max_px,
            secondary_scroll_max_px: presentation.secondary_scroll_max_px,
        };
        match presentation.panel_kind {
            replay_core::CombatPanelKindExact::LandCombat => {
                panels.push(available(Xui20ViewerPanel::LandCombat {
                    combat_id: presentation
                        .combat_id
                        .ok_or("XUI20 validated land panel lost combat id")?,
                    generation: presentation
                        .combat_generation
                        .ok_or("XUI20 validated land panel lost generation")?,
                }))
            }
            replay_core::CombatPanelKindExact::NavalCombat => {
                panels.push(available(Xui20ViewerPanel::NavalCombat {
                    combat_id: presentation
                        .combat_id
                        .ok_or("XUI20 validated naval panel lost combat id")?,
                    generation: presentation
                        .combat_generation
                        .ok_or("XUI20 validated naval panel lost generation")?,
                }))
            }
            replay_core::CombatPanelKindExact::NavalResults => {
                panels.push(available(Xui20ViewerPanel::NavalResults {
                    combat_id: presentation
                        .combat_id
                        .ok_or("XUI20 validated naval-result panel lost combat id")?,
                    generation: presentation
                        .combat_generation
                        .ok_or("XUI20 validated naval-result panel lost generation")?,
                }))
            }
            replay_core::CombatPanelKindExact::CombatLog => {
                let country_tag = presentation
                    .owner_country_tag
                    .clone()
                    .ok_or("XUI20 validated combat-log panel lost country identity")?;
                for tab in [
                    Xui20CombatLogTab::Combats,
                    Xui20CombatLogTab::Equipment,
                    Xui20CombatLogTab::Templates,
                ] {
                    panels.push(available(Xui20ViewerPanel::CombatLog {
                        country_tag: country_tag.clone(),
                        tab,
                    }));
                }
            }
            replay_core::CombatPanelKindExact::NavalLosses => {
                panels.push(available(Xui20ViewerPanel::NavalLosses {
                    country_tag: presentation
                        .owner_country_tag
                        .clone()
                        .ok_or("XUI20 validated naval-loss panel lost country identity")?,
                }))
            }
        }
    }
    Ok(panels)
}

fn reconcile_xui20_viewer_to_candidate(
    viewer: &mut game_ui::combat_detail_canvas::Xui20ViewerState,
    candidate: Option<&Xui20PresentedFrame>,
    frame_hour: i64,
) {
    let Some(panel) = viewer.panel.as_ref() else {
        return;
    };
    let contract = candidate
        .filter(|candidate| i64::try_from(candidate.frame_hour).ok() == Some(frame_hour))
        .and_then(|candidate| {
            candidate
                .available_panels
                .iter()
                .find(|available| &available.panel == panel)
        });
    if let Some(contract) = contract {
        viewer.selected_hour = Some(frame_hour);
        viewer.primary_scroll_px = viewer
            .primary_scroll_px
            .clamp(0, contract.primary_scroll_max_px);
        viewer.secondary_scroll_px = viewer
            .secondary_scroll_px
            .clamp(0, contract.secondary_scroll_max_px);
    } else {
        viewer.reset_for_seek();
    }
}

fn rollback_xui20_staged_candidate(
    viewer: &mut game_ui::combat_detail_canvas::Xui20ViewerState,
    viewer_checkpoint: &mut Option<game_ui::combat_detail_canvas::Xui20ViewerState>,
    cursor: &mut Option<(i32, i32)>,
    cursor_checkpoint: &mut Option<(i32, i32)>,
    pending: &mut Option<Xui20PresentedFrame>,
    pending_staged: &mut bool,
) -> Result<()> {
    *viewer = viewer_checkpoint
        .take()
        .ok_or("XUI20 viewer checkpoint is absent after semantic preparation refusal")?;
    *cursor = cursor_checkpoint.take();
    *pending = None;
    *pending_staged = false;
    Ok(())
}

fn commit_xui08_staged_candidate<T>(
    pending: &mut Option<T>,
    pending_staged: &mut bool,
    viewer_checkpoint: &mut Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
) -> Result<Option<T>> {
    if !*pending_staged {
        return Ok(None);
    }
    viewer_checkpoint
        .take()
        .ok_or("XUI08 viewer checkpoint is absent for presented frame")?;
    *pending_staged = false;
    Ok(pending.take())
}

fn reconcile_xui08_candidate(
    viewer: &mut game_ui::peace_conference_ui::PeaceConferenceViewerState,
    candidate: Option<&Xui08PresentedFrame>,
) {
    if let Some(candidate) = candidate {
        viewer.synchronize(&candidate.admitted);
        if !candidate.admitted.active() {
            viewer.set_open(false);
        }
    } else {
        viewer.suspend_unavailable_frame();
    }
}

fn stage_xui08_viewer_input<'a>(
    pending_input: &'a mut Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
    presented: &game_ui::peace_conference_ui::PeaceConferenceViewerState,
) -> &'a mut game_ui::peace_conference_ui::PeaceConferenceViewerState {
    pending_input.get_or_insert_with(|| presented.clone())
}

fn xui08_current_viewer_state<'a>(
    pending_input: &'a Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
    presented: &'a game_ui::peace_conference_ui::PeaceConferenceViewerState,
) -> &'a game_ui::peace_conference_ui::PeaceConferenceViewerState {
    pending_input.as_ref().unwrap_or(presented)
}

fn release_xui08_pointer_capture(
    pending_input: &mut Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
    presented: &game_ui::peace_conference_ui::PeaceConferenceViewerState,
) -> bool {
    stage_xui08_viewer_input(pending_input, presented).release_active_scrollbar()
}

fn release_xui08_pointer_capture_durably(
    pending_input: &mut Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
    presented: &mut game_ui::peace_conference_ui::PeaceConferenceViewerState,
) -> bool {
    let released_presented = presented.release_active_scrollbar();
    let released_pending = pending_input
        .as_mut()
        .is_some_and(|pending| pending.release_active_scrollbar());
    released_presented || released_pending
}

fn xui08_blocks_lower_pointer_input_state(
    current: &game_ui::peace_conference_ui::PeaceConferenceViewerState,
    input_region: Option<&game_ui::peace_conference_ui::PeaceConferenceInputRegion>,
    x: i32,
    y: i32,
) -> bool {
    current.open && input_region.is_some_and(|region| region.panel_contains((x, y)))
}

fn stage_xui08_closed(
    pending_input: &mut Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
    presented: &game_ui::peace_conference_ui::PeaceConferenceViewerState,
) {
    stage_xui08_viewer_input(pending_input, presented).set_open(false);
}

fn begin_xui08_staged_candidate(
    viewer: &mut game_ui::peace_conference_ui::PeaceConferenceViewerState,
    pending_input: &mut Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
    viewer_checkpoint: &mut Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
) -> Result<()> {
    if viewer_checkpoint.is_some() {
        return Err("XUI08 viewer checkpoint already exists before execute".into());
    }
    *viewer_checkpoint = Some(viewer.clone());
    if let Some(staged_input) = pending_input.take() {
        *viewer = staged_input;
    }
    Ok(())
}

fn close_xui08_topmost_state(
    state: &mut game_ui::peace_conference_ui::PeaceConferenceViewerState,
) -> bool {
    if !state.open {
        return false;
    }
    if state.selected_allocation_id.is_some() {
        state.clear_contested_popup();
    } else {
        state.set_open(false);
    }
    true
}

fn rollback_xui08_staged_candidate<T>(
    viewer: &mut game_ui::peace_conference_ui::PeaceConferenceViewerState,
    viewer_checkpoint: &mut Option<game_ui::peace_conference_ui::PeaceConferenceViewerState>,
    pending: &mut Option<T>,
    pending_staged: &mut bool,
) -> Result<()> {
    *viewer = viewer_checkpoint
        .take()
        .ok_or("XUI08 viewer checkpoint is absent for rejected frame")?;
    *pending = None;
    *pending_staged = false;
    Ok(())
}

#[derive(Clone, Debug)]
struct Xui23PresentedFrame {
    /// A RESET KEY, not a name: the presented frame is discarded when the
    /// viewer's selection changes, and the type is a key precisely because no
    /// accessor spells it.
    country_tag: game_ui::stored_country_selection::StoredCountrySelection,
    frame_hour: i64,
    viewer_clock_micros: u64,
    animation_elapsed_micros: u64,
    contexts: Vec<game_ui::active_abilities::ObserverAbilityContext>,
    input_region: Option<game_ui::active_abilities::ObserverAbilityInputRegion>,
    button_regions: Vec<game_ui::active_abilities::ObserverAbilityButtonRegion>,
}

#[derive(Clone, Debug)]
struct Xui15InteractionSurface {
    panel: Xui15InteractionPanel,
    viewport: [usize; 2],
}

#[derive(Clone, Debug)]
enum Xui15InteractionPanel {
    Production(game_ui::trade_compositor::StrictXui15PanelInput),
    #[cfg(test)]
    Fixture,
}

impl Xui15InteractionSurface {
    fn production(
        panel: game_ui::trade_compositor::StrictXui15PanelInput,
        viewport: [usize; 2],
    ) -> Self {
        Self {
            panel: Xui15InteractionPanel::Production(panel),
            viewport,
        }
    }

    fn panel(&self) -> Result<&game_ui::trade_compositor::StrictXui15PanelInput> {
        match &self.panel {
            Xui15InteractionPanel::Production(panel) => Ok(panel),
            #[cfg(test)]
            Xui15InteractionPanel::Fixture => {
                Err("test XUI15 surface has no compositor panel".into())
            }
        }
    }

    #[cfg(test)]
    fn fixture(viewport: [usize; 2]) -> Self {
        Self {
            panel: Xui15InteractionPanel::Fixture,
            viewport,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ObserverUiPresentTransactionOwner {
    viewer: Option<ObserverViewerPresentTransaction>,
    xui15_presented: Option<Xui15InteractionSurface>,
    xui15_pending: Option<Xui15InteractionSurface>,
    xui15_checkpoint: Option<Option<Xui15InteractionSurface>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObserverUiFrameDisposition {
    Presented,
    Skipped,
    Occluded,
    ExecuteError,
}

impl ObserverUiPresentTransactionOwner {
    fn begin(
        &mut self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        if self.viewer.is_some() || self.xui15_checkpoint.is_some() {
            return Err("observer UI Present transaction already has a pending disposition".into());
        }
        self.viewer = Some(ObserverViewerPresentTransaction::begin(viewer));
        self.xui15_checkpoint = Some(self.xui15_presented.clone());
        Ok(())
    }

    fn stage_xui15(&mut self, candidate: Xui15InteractionSurface) {
        self.xui15_pending = Some(candidate);
    }

    fn presented_xui15(&self) -> Option<&Xui15InteractionSurface> {
        self.xui15_presented.as_ref()
    }

    fn viewer_checkpoint_mut(
        &mut self,
    ) -> Option<&mut game_ui::production_renderer::ObserverViewerUiState> {
        self.viewer
            .as_mut()
            .map(|transaction| &mut transaction.checkpoint)
    }

    fn has_pending_disposition(&self) -> bool {
        self.viewer.is_some() || self.xui15_checkpoint.is_some()
    }

    fn invalidate_runtime_surfaces(&mut self) {
        self.xui15_presented = None;
        self.xui15_pending = None;
    }

    fn make_resize_invalidation_durable(
        &mut self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        let transaction = self
            .viewer
            .as_mut()
            .ok_or("observer UI resize has no active Present transaction")?;
        transaction.make_politics_frame_invalidation_durable(viewer);
        transaction.make_resize_pointer_release_durable(viewer);
        let checkpoint = self
            .xui15_checkpoint
            .as_mut()
            .ok_or("observer UI resize has no XUI15 Present checkpoint")?;
        *checkpoint = self.xui15_presented.clone();
        Ok(())
    }

    fn finish_presented(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        production_lines_scroll_offset: Option<i32>,
    ) -> Result<()> {
        self.xui15_presented = self.xui15_pending.take();
        self.xui15_checkpoint
            .take()
            .ok_or("XUI15 Presented transaction has no checkpoint")?;
        self.viewer
            .take()
            .ok_or("observer viewer Presented transaction has no checkpoint")?
            .commit();
        if let Some(offset) = production_lines_scroll_offset {
            viewer.production_lines.scroll_offset_px = offset;
        }
        Ok(())
    }

    fn reject(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        self.xui15_pending = None;
        self.xui15_presented = self
            .xui15_checkpoint
            .take()
            .ok_or("XUI15 rejected transaction has no checkpoint")?;
        self.viewer
            .take()
            .ok_or("observer viewer rejected transaction has no checkpoint")?
            .reject(viewer);
        Ok(())
    }

    fn finish_disposition(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        disposition: ObserverUiFrameDisposition,
        production_lines_scroll_offset: Option<i32>,
    ) -> Result<()> {
        match disposition {
            ObserverUiFrameDisposition::Presented => {
                self.finish_presented(viewer, production_lines_scroll_offset)
            }
            ObserverUiFrameDisposition::Skipped
            | ObserverUiFrameDisposition::Occluded
            | ObserverUiFrameDisposition::ExecuteError => self.reject(viewer),
        }
    }
}

fn apply_xui13_feature_local_composition(
    composition: std::result::Result<
        game_ui::designer_observer::ObserverDesignerSurface,
        game_ui::production_renderer::ProductionUiRejection,
    >,
    framebuffer: &mut game_ui::dds::RgbaImage,
    country_tag: &str,
    measured_empty: bool,
    authored_panel_draw_count: &mut usize,
    semantic_receipts: &mut Vec<String>,
    output_gaps: &mut Vec<game_ui::production_renderer::ObserverUiGap>,
    feature_local_gaps: &mut Vec<String>,
) -> Result<bool> {
    match composition {
        Ok(designer_surface) => {
            game_ui::designer_observer::blend(framebuffer, &designer_surface.output.image)?;
            *authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
            semantic_receipts.push(format!(
                "XUI13:country={}:family={:?}:rows={}:visible={}:measured_empty={}:manager-overlay-composited",
                country_tag,
                designer_surface.family,
                designer_surface.ordered_design_ids.len(),
                designer_surface.visible_design_ids.len(),
                measured_empty && designer_surface.membership_complete,
            ));
            for (field, reason) in designer_surface.field_refusals {
                let gap = game_ui::production_renderer::ObserverUiGap {
                    surface: "XUI13/designer", field: "row_or_subtree",
                    required_domain: "exact-hour native inputs and authenticated authored designer laws",
                    reason: format!("{field}: {reason}"),
                };
                feature_local_gaps.push(format!("XUI13:field={field}:domain={}:unavailable={}", gap.required_domain, gap.reason));
                output_gaps.push(gap);
            }
            Ok(true)
        }
        Err(error) => {
            let gap = game_ui::production_renderer::ObserverUiGap {
                surface: "XUI13/designer",
                field: "effective_content_or_compositor_capability",
                required_domain:
                    "exact-hour A27 native inputs, D53 intelligence and effective authored designer roots",
                reason: format!("XUI13 feature-local composition rejected: {error:?}"),
            };
            feature_local_gaps.push(format!(
                "XUI13:field={}:domain={}:unavailable={}",
                gap.field, gap.required_domain, gap.reason
            ));
            output_gaps.push(gap);
            Ok(false)
        }
    }
}

fn apply_xui15_feature_local_composition(
    composition: std::result::Result<
        game_ui::compose::ComposeOutput,
        game_ui::production_renderer::ProductionUiRejection,
    >,
    framebuffer: &mut game_ui::dds::RgbaImage,
    panel: &game_ui::trade_compositor::StrictXui15PanelInput,
    authored_panel_draw_count: &mut usize,
    semantic_receipts: &mut Vec<String>,
    output_gaps: &mut Vec<game_ui::production_renderer::ObserverUiGap>,
    feature_local_gaps: &mut Vec<String>,
) -> Result<bool> {
    match composition {
        Ok(output) => {
            game_ui::designer_observer::blend(framebuffer, &output.image)?;
            *authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
            semantic_receipts.push(format!(
                "XUI15:country={}:rows={}:filters={}:completeness={}:measured_empty={}:observer-overlay-composited",
                panel.country_tag(),
                panel.catalogue_row_count(),
                panel.country_filter_count(),
                panel.completeness_record_sha256(),
                panel.measured_empty(),
            ));
            Ok(true)
        }
        Err(error) => {
            let gap = game_ui::production_renderer::ObserverUiGap {
                surface: "XUI15/trade",
                field: "effective_content_or_compositor_capability",
                required_domain:
                    "strict frame-resident producer authority plus effective authored four-grid root",
                reason: format!("XUI15 feature-local composition rejected: {error:?}"),
            };
            feature_local_gaps.push(format!(
                "XUI15:field={}:domain={}:unavailable={}",
                gap.field, gap.required_domain, gap.reason
            ));
            output_gaps.push(gap);
            Ok(false)
        }
    }
}

impl ProductObserverUiRuntime {
    pub fn begin_observer_frame_transaction(
        &mut self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        if self.observer_frame_checkpoint.is_some() {
            return Err("observer UI previous frame checkpoint has no Present disposition".into());
        }
        let checkpoint = ObserverUiFrameCheckpoint::capture(viewer, self)?;
        self.present_transaction.begin(viewer)?;
        self.observer_frame_checkpoint = Some(checkpoint);
        Ok(())
    }

    pub fn from_playback_content(
        gfx: &Gfx,
        resolver: &map_viewer::GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        playback_profile: &game_ui::variant_admission::PlaybackUiProfile,
        packaged_decision_catalog: Option<&replay_core::PackagedDecisionCatalog>,
        static_research_catalog: Arc<game_ui::static_research::StaticResearchUiCatalog>,
        static_focus_definitions: Arc<replay_core::AdmittedStaticFocusDefinitions>,
    ) -> Result<Self> {
        let assets = AssetResolver::from_content_admission(resolver, content_admission)?;
        let runtime_mod_ui = game_ui::mod_ui_discovery::discover_admitted_runtime_mod_ui(
            &assets,
            playback_profile.localisation_language(),
        )?;
        let mut initialization_receipts = vec![format!(
            "XUI24:gui={}:gfx={}:loc={}:panels={}:mod_layer_gui={}",
            runtime_mod_ui.gui_files,
            runtime_mod_ui.gfx_files,
            runtime_mod_ui.localisation_files,
            runtime_mod_ui.panels.len(),
            runtime_mod_ui.mod_layer_gui_files,
        )];
        let mut initialization_gaps = Vec::new();
        let decision_catalog = packaged_decision_catalog
            .map(|packaged| {
                game_ui::decision_replay::DecisionStaticCatalog::from_extractor_json(
                    packaged.bytes(),
                )
            })
            .transpose()?;
        let authored_observer_ui = crate::gui_definition_admission::AuthoredObserverUi::new(
            resolver,
            content_admission,
            playback_profile.localisation_language(),
        )?;
        initialization_receipts.push(format!(
            "XUI01/XUI02/XUI04/XUI07/XUI24/XUI27:manifest-bound-effective-windows={}",
            authored_observer_ui.discovered_window_ids().count()
        ));
        let frontend_dispositions =
            authored_observer_ui.frontend_family_dispositions(content_admission)?;
        let admitted_frontend_families =
            authored_observer_ui.admitted_frontend_family_ids(content_admission)?;
        if frontend_dispositions.len() != admitted_frontend_families.len() {
            return Err("XUI26 disposition census differs from admitted family inventory".into());
        }
        initialization_receipts.push(format!(
            "XUI26:manifest-bound-effective-families={}",
            frontend_dispositions.len()
        ));
        let scope_count = |scope| {
            frontend_dispositions
                .iter()
                .filter(|row| row.scope == scope)
                .count()
        };
        initialization_receipts.push(format!(
            "XUI26:family-scope:static_shipping={}:replay_payload_shipping={}:host_runtime_nonshipping={}:external_service_nonshipping={}",
            scope_count(game_ui::frontend_meta::FrontendFamilyScope::StaticShipping),
            scope_count(game_ui::frontend_meta::FrontendFamilyScope::ReplayPayloadShipping),
            scope_count(game_ui::frontend_meta::FrontendFamilyScope::HostRuntimeNonshipping),
            scope_count(game_ui::frontend_meta::FrontendFamilyScope::ExternalServiceNonshipping),
        ));
        for disposition in frontend_dispositions {
            if matches!(
                disposition.scope,
                game_ui::frontend_meta::FrontendFamilyScope::HostRuntimeNonshipping
                    | game_ui::frontend_meta::FrontendFamilyScope::ExternalServiceNonshipping
            ) {
                initialization_receipts.push(format!(
                    "XUI26:family={}:window={}:source={}:scope={:?}",
                    disposition.family_id,
                    disposition.window_id,
                    disposition.source_file,
                    disposition.scope,
                ));
                continue;
            }
            if disposition.unavailable_bindings.is_empty() {
                initialization_receipts.push(format!(
                    "XUI26:family={}:window={}:source={}:static=available",
                    disposition.family_id, disposition.window_id, disposition.source_file,
                ));
                continue;
            }
            for (slot_id, reason_code) in disposition.unavailable_bindings {
                initialization_gaps.push(format!(
                    "XUI26:family={}:window={}:source={}:slot={}:unavailable={}",
                    disposition.family_id,
                    disposition.window_id,
                    disposition.source_file,
                    slot_id,
                    reason_code,
                ));
            }
        }
        let authored_observer_ui = Some(authored_observer_ui);
        let scripted_gui_registry =
            match game_ui::content_interpreter::ProductionContentInterpreter::new(
                resolver,
                content_admission,
                playback_profile.localisation_language(),
            )
            .and_then(|interpreter| interpreter.load_scripted_gui_registry())
            {
                Ok(registry) => {
                    initialization_receipts.push(format!(
                        "XUI22:effective_scripted_gui_files={}",
                        registry.provenance.len()
                    ));
                    Some(registry)
                }
                Err(error) => {
                    initialization_gaps.push(format!(
                        "XUI22 effective scripted-GUI registry is unavailable: {error}"
                    ));
                    None
                }
            };
        let renderer = game_ui::production_renderer::ProductionUiRenderer::new_for_playback(
            assets,
            content_admission,
            playback_profile,
        )?;
        let static_focus_content = Arc::new(
            game_ui::static_focus_defs::AdmittedStaticFocusUiContent::new(
                static_focus_definitions,
                content_admission.manifest_digest_sha256(),
            )?,
        );
        Self::new(
            gfx,
            renderer,
            game_ui::production_renderer::ObserverPlaybackUiProfile {
                variant_key: playback_profile.key().clone(),
            },
            static_research_catalog,
            static_focus_content,
            scripted_gui_registry,
            authored_observer_ui,
            content_admission.clone(),
            decision_catalog,
            initialization_receipts,
            initialization_gaps,
        )
    }

    fn new(
        gfx: &Gfx,
        renderer: game_ui::production_renderer::ProductionUiRenderer,
        playback: game_ui::production_renderer::ObserverPlaybackUiProfile,
        static_research_catalog: Arc<game_ui::static_research::StaticResearchUiCatalog>,
        static_focus_content: Arc<game_ui::static_focus_defs::AdmittedStaticFocusUiContent>,
        scripted_gui_registry: Option<
            game_ui::content_interpreter::Interpreted<
                game_ui::scripted_gui_runtime::ScriptedGuiRegistry,
            >,
        >,
        authored_observer_ui: Option<crate::gui_definition_admission::AuthoredObserverUi>,
        content_admission: replay_core::ValidatedContentAdmission,
        decision_catalog: Option<game_ui::decision_replay::DecisionStaticCatalog>,
        initialization_receipts: Vec<String>,
        initialization_gaps: Vec<String>,
    ) -> Result<Self> {
        Ok(Self {
            renderer,
            image: ImageRenderer::new(gfx)?,
            playback,
            static_research_catalog,
            static_focus_content,
            scripted_gui_registry,
            authored_observer_ui,
            content_admission,
            decision_catalog,
            initialization_receipts,
            initialization_gaps,
            execute_lifecycle: ObserverUiExecuteLifecycle::default(),
            present_transaction: ObserverUiPresentTransactionOwner::default(),
            xui05_controller: Default::default(),
            xui05_pending_owner: None,
            xui05_pending_controller_checkpoint: None,
            xui23_presented: None,
            xui23_pending: None,
            xui10_presented: None,
            xui10_pending: None,
            xui10_drag_cursor: None,
            observer_frame_checkpoint: None,
            xui09_presented: None,
            xui09_pending: None,
            xui09_pending_staged: false,
            xui09_pending_viewer_checkpoint: None,
            xui09_drag_cursor: None,
            xui09_cursor: None,
            xui11_presented: None,
            xui11_history_anchor: None,
            xui11_pending: None,
            xui11_pending_staged: false,
            xui11_pending_viewer_checkpoint: None,
            xui11_cursor: None,
            xui18_presented: None,
            xui18_pending: None,
            xui18_pending_staged: false,
            xui18_pending_viewer_checkpoint: None,
            xui18_cursor: None,
            xui19_presented: None,
            xui19_history_anchor: None,
            xui19_pending: None,
            xui19_pending_staged: false,
            xui19_pending_viewer_checkpoint: None,
            xui19_cursor: None,
            xui08_presented: None,
            xui08_history_anchor: None,
            xui08_pending: None,
            xui08_frame_clock: Default::default(),
            xui08_pending_staged: false,
            xui08_pending_viewer_checkpoint: None,
            xui08_pending_viewer_input: None,
            xui08_cursor: None,
            xui14_presented: None,
            xui14_history_anchor: None,
            xui14_pending: None,
            xui14_pending_staged: false,
            xui14_pending_viewer_checkpoint: None,
            xui14_cursor: None,
            xui17_cursor: None,
            xui20_presented: None,
            xui20_history_anchor: None,
            xui20_pending: None,
            xui20_pending_staged: false,
            xui20_pending_viewer_checkpoint: None,
            xui20_pending_cursor_checkpoint: None,
            xui20_cursor: None,
            xui21_presented: None,
            xui21_pending: None,
            xui21_pending_staged: false,
            xui21_pending_viewer_checkpoint: None,
            xui21_cursor: None,
        })
    }

    pub fn execute(
        &mut self,
        gfx: &Gfx,
        world_frame: &WorldFrame,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        playback_speed_step: usize,
        playback_playing: bool,
        viewer_clock_micros: u64,
        production_lines_inputs: ProductionLinesUiSupplementalInputs,
        occupation_outcome: Option<&ObserverOccupationOutcome>,
    ) -> Result<ObserverUiOutcome> {
        if self.observer_frame_checkpoint.is_none() {
            return Err(
                "observer UI frame transaction was not started before event dispatch".into(),
            );
        }
        self.execute_lifecycle.begin()?;
        if self.xui23_pending.is_some() {
            return Err("XUI23 previous staged frame has no Present disposition".into());
        }
        if self.xui10_pending.is_some() {
            return Err("XUI10 previous staged frame has no Present disposition".into());
        }
        if self.xui09_pending_staged {
            return Err("XUI09 previous staged frame has no Present disposition".into());
        }
        if self.xui11_pending_staged {
            return Err("XUI11 previous staged frame has no Present disposition".into());
        }
        if self.xui18_pending_staged {
            return Err("XUI18 previous staged frame has no Present disposition".into());
        }
        if self.xui19_pending_staged {
            return Err("XUI19 previous staged frame has no Present disposition".into());
        }
        if self.xui08_pending_staged {
            return Err("XUI08 previous staged frame has no Present disposition".into());
        }
        if self.xui14_pending_staged {
            return Err("XUI14 previous staged frame has no Present disposition".into());
        }
        if self.xui20_pending_staged {
            return Err("XUI20 previous staged frame has no Present disposition".into());
        }
        if self.xui21_pending_staged {
            return Err("XUI21 previous staged frame has no Present disposition".into());
        }
        let reset_xui19 = self.xui19_presented.as_ref().is_some_and(|prior| {
            let country_changed =
                !viewer.selected_country.is(prior.country_tag.as_str());
            let hour = world_frame.hour_index();
            let timeline_discontinuity =
                hour != prior.frame_hour && hour != prior.frame_hour.saturating_add(1);
            country_changed || timeline_discontinuity
        });
        if reset_xui19 {
            viewer.decision_panel_open = false;
            viewer.decision_scroll_px = 0;
            viewer.collapsed_decision_categories.clear();
            self.xui19_presented = None;
            self.xui19_history_anchor = None;
            self.xui19_pending = None;
            self.xui19_pending_staged = false;
            self.xui19_pending_viewer_checkpoint = None;
            self.xui19_cursor = None;
        }
        if self.xui23_presented.is_none() {
            if let Some(prior) = self.xui18_presented.as_ref() {
                let country_changed =
                    !viewer.selected_country.is(prior.country_tag.as_str());
                let current_hour = u64::try_from(world_frame.hour_index()).ok();
                let timeline_discontinuity = current_hour != Some(prior.frame_hour)
                    && current_hour != Some(prior.frame_hour.saturating_add(1));
                if timeline_discontinuity {
                    self.reset_xui23_for_seek(viewer);
                } else if country_changed {
                    self.reset_country_dependent_ui_for_country_change(viewer);
                }
            }
        }
        if let Some(prior) = self.xui23_presented.as_ref() {
            let country_changed = viewer.selected_country != prior.country_tag;
            let timeline_discontinuity = world_frame.hour_index() != prior.frame_hour
                && world_frame.hour_index() != prior.frame_hour.saturating_add(1);
            if timeline_discontinuity {
                self.reset_xui23_for_seek(viewer);
            } else if country_changed {
                self.reset_country_dependent_ui_for_country_change(viewer);
            }
        }
        if let Some(authored) = self.authored_observer_ui.as_mut() {
            authored.reconcile_frontend_frame(world_frame)?;
        }
        self.xui09_pending_viewer_checkpoint = Some(viewer.research.clone());
        self.xui11_pending_viewer_checkpoint = Some(viewer.politics.clone());
        self.xui18_pending_viewer_checkpoint = Some(viewer.power_balance.clone());
        self.xui19_pending_viewer_checkpoint = Some((
            viewer.decision_panel_open,
            viewer.decision_scroll_px,
            viewer.collapsed_decision_categories.clone(),
        ));
        let mut xui19_preflight_gap = None;
        let mut xui19_candidate = xui19_passive_candidate(
            world_frame,
            &viewer.selected_country,
            &mut xui19_preflight_gap,
        );
        if xui19_candidate.as_ref().is_some_and(|candidate| {
            self.xui19_history_anchor
                .as_ref()
                .is_some_and(|anchor| !anchor.accepts(candidate))
        }) {
            xui19_preflight_gap = Some(
                "XUI19:field=history:domain=_r2_decision_evaluation_exact:unavailable=NON_CONTIGUOUS_OR_REWRITTEN_PASSIVE_RECORD"
                    .to_string(),
            );
            xui19_candidate = None;
        }
        self.xui19_pending = xui19_candidate;
        self.xui19_pending_staged = true;
        if self.xui19_pending.is_none() && viewer.decision_panel_open {
            viewer.decision_panel_open = false;
            viewer.decision_scroll_px = 0;
            viewer.collapsed_decision_categories.clear();
            self.xui19_cursor = None;
        }
        // cannot leave hidden XUI08 input or tooltip ownership underneath.
        if viewer.intelligence.panel_open {
            self.close_xui08(viewer);
        }
        begin_xui08_staged_candidate(
            &mut viewer.peace_conference,
            &mut self.xui08_pending_viewer_input,
            &mut self.xui08_pending_viewer_checkpoint,
        )?;
        self.xui14_pending_viewer_checkpoint = Some(viewer.intelligence.clone());
        self.xui20_pending_viewer_checkpoint = Some(viewer.combat.clone());
        self.xui20_pending_cursor_checkpoint = self.xui20_cursor;
        begin_xui21_staged_candidate(
            &viewer.events,
            &mut self.xui21_pending_viewer_checkpoint,
            &self.xui21_pending,
            self.xui21_pending_staged,
        )?;
        // THE SELECTION, which is what the panel stores: no spelling passes
        // through, and the census question the panel needs is asked where the
        // panel is composed, against this frame.
        viewer.politics.reconcile_country(&viewer.selected_country.clone());
        viewer
            .intelligence
            .reconcile_country(&viewer.selected_country.clone());
        let xui18_carrier_candidate = viewer
            .selected_country
            .named_in(world_frame)
            .map(|tag| tag.tag())
            .and_then(|country| {
            let state = world_frame.power_balance_state_exact().state.as_ref()?;
            let presentation = world_frame
                .power_balance_presentation_exact()
                .presentation
                .as_ref()?;
            if state.frame_hour != presentation.frame_hour
                || state.semantic_receipt_sha256 != presentation.state_semantic_receipt_sha256
                || !state.countries.iter().any(|row| row.country_tag == country)
            {
                return None;
            }
            Some(Xui18PresentedFrame {
                country_tag: country.to_string(),
                frame_hour: state.frame_hour,
                state_semantic_receipt_sha256: state.semantic_receipt_sha256.clone(),
                availability: Xui18PresentedAvailability::MeasuredEmpty,
                panel_region: None,
                close_region: None,
                tooltip_targets: Vec::new(),
            })
        });
        let mut xui18_preflight_gap = None;
        let xui18_candidate = xui18_carrier_candidate.and_then(|mut candidate| {
            let admitted = match (
                self.renderer.power_balance_catalog(),
                self.renderer.power_balance_decisions(),
            ) {
                (Ok(catalog), Ok(decisions)) => {
                    crate::power_balance_runtime::admit_country_power_balance_ui(
                        world_frame,
                        &candidate.country_tag,
                        catalog,
                        decisions,
                        self.renderer.content_manifest_digest_sha256(),
                    )
                }
                (Err(error), _) | (_, Err(error)) => Err(format!(
                    "effective authored power-balance catalog is unavailable: {error}"
                )),
            };
            match admitted {
                Ok(game_ui::power_balance::AdmittedPowerBalanceScreen::Visible(panels))
                    if panels.len() == 1 =>
                {
                    candidate.availability = Xui18PresentedAvailability::Visible;
                    Some(candidate)
                }
                Ok(game_ui::power_balance::AdmittedPowerBalanceScreen::Visible(panels)) => {
                    xui18_preflight_gap = Some(format!(
                        "XUI18:field=semantic_admission:domain=passive_all_country_power_balance:unavailable=EXPECTED_ONE_VISIBLE_ROW_GOT_{}",
                        panels.len()
                    ));
                    None
                }
                Ok(game_ui::power_balance::AdmittedPowerBalanceScreen::HiddenNoActiveBalance) => {
                    candidate.availability = Xui18PresentedAvailability::MeasuredEmpty;
                    Some(candidate)
                }
                Err(error) => {
                    xui18_preflight_gap = Some(format!(
                        "XUI18:field=semantic_admission:domain=passive_all_country_power_balance:unavailable={error}"
                    ));
                    None
                }
            }
        });
        self.xui18_pending_staged = true;
        self.xui18_pending = xui18_candidate;
        if !self
            .xui18_pending
            .as_ref()
            .is_some_and(Xui18PresentedFrame::has_active_balance)
            && viewer.power_balance.panel_open
        {
            viewer.power_balance.close();
            self.xui18_cursor = None;
        }
        if viewer
            .combat
            .panel
            .as_ref()
            .and_then(|panel| panel.country_tag())
            .is_some_and(|country| !viewer.selected_country.is(country))
        {
            viewer.combat.reset_for_seek();
            self.xui20_cursor = None;
        }
        let mut xui20_preflight_gaps = Vec::new();
        let xui20_candidate = world_frame
            .combat_presentation_exact()
            .presentation
            .as_ref()
            .and_then(|carrier| {
                let continuity_ok = self
                    .xui20_history_anchor
                    .as_ref()
                    .is_none_or(|prior| {
                        prior.accepts(
                            carrier.frame_hour,
                            carrier.capture_sequence,
                            carrier.previous_record_sha256.as_deref(),
                            &carrier.record_sha256,
                        )
                    });
                if !continuity_ok {
                    xui20_preflight_gaps.push(
                        "XUI20:field=history:domain=_r2_combat_presentation_exact:unavailable=NON_CONTIGUOUS_OR_REWRITTEN_PASSIVE_RECORD"
                            .into(),
                    );
                    return None;
                }
                match xui20_available_panels(carrier) {
                    Ok(available_panels) => Some(Xui20PresentedFrame {
                        frame_hour: carrier.frame_hour,
                        capture_sequence: carrier.capture_sequence,
                        record_sha256: carrier.record_sha256.clone(),
                        available_panels,
                        input_region: None,
                        tooltip_targets: Vec::new(),
                    }),
                    Err(error) => {
                        xui20_preflight_gaps.push(format!(
                            "XUI20:field=panel_identity:domain=_r2_combat_presentation_exact:unavailable={error}"
                        ));
                        None
                    }
                }
            });
        if xui20_candidate.is_none() {
            xui20_preflight_gaps.push(format!(
                "XUI20:field=passive_global_presentation:domain=_r2_combat_presentation_exact:unavailable={:?}",
                world_frame.combat_presentation_exact_status()
            ));
        }
        let had_combat_panel = viewer.combat.panel.is_some();
        reconcile_xui20_viewer_to_candidate(
            &mut viewer.combat,
            xui20_candidate.as_ref(),
            world_frame.hour_index(),
        );
        if had_combat_panel && viewer.combat.panel.is_none() {
            self.xui20_cursor = None;
        }
        self.xui20_pending_staged = true;
        self.xui20_pending = xui20_candidate;
        if viewer.politics.panel_open {
            if let Some(input) = self
                .xui11_presented
                .as_ref()
                .filter(|presented| {
                    viewer.selected_country.is(presented.country_tag.as_str())
                })
                .and_then(|presented| presented.input_region)
            {
                viewer
                    .politics
                    .advance_smooth_scroll(input.smooth_factor_fixed)?;
            }
        }
        if self.xui10_presented.as_ref().is_some_and(|prior| {
            !viewer.selected_country.is(prior.country_tag.as_str())
                || i64::try_from(prior.frame_hour).ok() != Some(world_frame.hour_index())
        }) {
            self.xui10_presented = None;
            self.xui10_drag_cursor = None;
            viewer.focus.selected_focus_id = None;
        }
        let mut xui22_preflight_gap = None;
        let xui22_panel = match (
            viewer.selected_country.is_selected(),
            self.scripted_gui_registry.as_ref(),
        ) {
            (true, Some(registry)) => match viewer.selected_country.named_in(world_frame) {
                Some(country) => {
                    match game_ui::observer_panels::admit_scripted_gui_observer_panel(
                        registry,
                        world_frame,
                        country,
                    ) {
                        Ok(panel) => Some(panel),
                        Err(error) => {
                            xui22_preflight_gap = Some(format!(
                                "XUI22:field=d84_effective_provider_join:domain=scripted_gui_runtime+effective_files:unavailable={error}"
                            ));
                            None
                        }
                    }
                }
                None => {
                    xui22_preflight_gap = Some(
                        "XUI22:field=selected_country:domain=contention_census:unavailable=SELECTED_COUNTRY_IS_A_CONTENDED_KEY"
                            .to_string(),
                    );
                    None
                }
            },
            (true, None) => {
                xui22_preflight_gap = Some(
                    "XUI22:field=effective_scripted_gui_registry:domain=effective common/scripted_guis/*.txt:unavailable=REGISTRY_NOT_ADMITTED"
                        .to_string(),
                );
                None
            }
            (false, _) => None,
        };
        let xui22_catalog = self
            .authored_observer_ui
            .as_ref()
            .map(|authored| authored.catalog());
        let mut semantic = match self
            .renderer
            .prepare_observer_semantic_frame_with_xui12_and_xui22_producers(
                world_frame,
                self.decision_catalog.as_ref(),
                self.static_focus_content.as_ref(),
                viewer,
                &self.playback,
                playback_speed_step,
                playback_playing,
                gfx.width as usize,
                gfx.height as usize,
                production_lines_inputs.producer(),
                xui22_catalog,
                xui22_panel.as_ref(),
            ) {
            Ok(semantic) => semantic,
            Err(error) => {
                rollback_xui20_staged_candidate(
                    &mut viewer.combat,
                    &mut self.xui20_pending_viewer_checkpoint,
                    &mut self.xui20_cursor,
                    &mut self.xui20_pending_cursor_checkpoint,
                    &mut self.xui20_pending,
                    &mut self.xui20_pending_staged,
                )?;
                return Err(format!("observer UI semantic preparation refused: {error:?}").into());
            }
        };
        let mut xui15_smoothing_rejection = None;
        if semantic
            .trade
            .as_ref()
            .and_then(|surface| surface.canvas.as_ref())
            .is_some()
        {
            match self.renderer.advance_xui15_trade_smooth_scroll(
                &mut viewer.trade,
                gfx.width as usize,
                gfx.height as usize,
            ) {
                Ok(()) => {
                    semantic = match self
                        .renderer
                        .prepare_observer_semantic_frame_with_xui12_and_xui22_producers(
                            world_frame,
                            self.decision_catalog.as_ref(),
                            self.static_focus_content.as_ref(),
                            viewer,
                            &self.playback,
                            playback_speed_step,
                            playback_playing,
                            gfx.width as usize,
                            gfx.height as usize,
                            production_lines_inputs.producer(),
                            xui22_catalog,
                            xui22_panel.as_ref(),
                        ) {
                        Ok(semantic) => semantic,
                        Err(error) => {
                            rollback_xui20_staged_candidate(
                                &mut viewer.combat,
                                &mut self.xui20_pending_viewer_checkpoint,
                                &mut self.xui20_cursor,
                                &mut self.xui20_pending_cursor_checkpoint,
                                &mut self.xui20_pending,
                                &mut self.xui20_pending_staged,
                            )?;
                            return Err(format!(
                                "observer UI semantic resealing refused: {error:?}"
                            )
                            .into());
                        }
                    };
                }
                Err(error) => xui15_smoothing_rejection = Some(error),
            }
        }
        viewer.theatre_settings_land_id = semantic.theatre_settings_land_id;
        if let Some(resolved) = semantic
            .designer
            .as_ref()
            .and_then(|surface| surface.viewer.as_ref())
        {
            viewer.designer = Some(resolved.clone());
        }
        let authored_source_windows = game_ui::production_renderer::observer_authored_hud_sources();
        self.authored_observer_ui
            .as_ref()
            .ok_or("XUI01 authored observer catalogue disappeared after initialization")?
            .validate_composed_roots(&authored_source_windows)?;
        let trade_semantic_state = semantic
            .trade
            .as_ref()
            .and_then(|trade| trade.screen.clone());
        let mut trade_available = semantic
            .trade
            .as_ref()
            .is_some_and(|surface| surface.canvas.is_some());
        let mut trade_measured_empty = semantic
            .trade
            .as_ref()
            .is_some_and(|surface| surface.canvas.is_some() && surface.measured_empty);
        let production_lines_open = semantic.production_lines.is_some();
        let production_lines_country = semantic
            .production_lines
            .as_ref()
            .and_then(|surface| surface.country_tag.clone());
        let mut production_lines_available = semantic
            .production_lines
            .as_ref()
            .is_some_and(|surface| surface.canvas.is_some());
        let designer_open = semantic.designer.is_some();
        let designer_country = semantic
            .designer
            .as_ref()
            .and_then(|surface| surface.country_tag.clone());
        let mut designer_available = semantic
            .designer
            .as_ref()
            .is_some_and(|surface| surface.viewer.is_some());
        let designer_measured_empty = semantic
            .designer
            .as_ref()
            .is_some_and(|surface| surface.viewer.is_some() && surface.measured_empty);
        let characters_open = semantic.characters.is_some();
        let characters_country = semantic
            .characters
            .as_ref()
            .and_then(|surface| surface.country_tag.clone());
        let mut characters_available = semantic
            .characters
            .as_ref()
            .is_some_and(|surface| surface.model.is_some() && surface.gaps.is_empty());
        let abilities_open = viewer.abilities.open;
        let abilities_country = semantic.abilities.country_tag.clone();
        let mut abilities_available =
            semantic.abilities.admitted.is_some() && semantic.abilities.gaps.is_empty();
        let abilities_measured_empty = semantic.abilities.measured_empty;
        let focus_open = viewer.focus.open;
        let focus_country = semantic.focus.country_tag().map(str::to_string);
        let focus_semantic_available = semantic.focus.semantic().is_some();
        let mut focus_pixels_available =
            semantic.focus.scene().is_some() && semantic.focus.gaps().is_empty();
        let focus_measured_empty = semantic.focus.measured_empty();
        let ability_animation_elapsed_micros = self
            .xui23_presented
            .as_ref()
            .filter(|prior| {
                viewer.selected_country == prior.country_tag
                    && (world_frame.hour_index() == prior.frame_hour
                        || world_frame.hour_index() == prior.frame_hour.saturating_add(1))
                    && viewer_clock_micros >= prior.viewer_clock_micros
            })
            .map(|prior| {
                prior
                    .animation_elapsed_micros
                    .saturating_add(viewer_clock_micros.saturating_sub(prior.viewer_clock_micros))
            })
            .unwrap_or_default();
        let mut output_gaps = semantic.gaps.clone();
        let decision_semantic_state = semantic.decisions.clone();
        let mut semantic_receipts = self.initialization_receipts.clone();
        let mut feature_local_gaps = self.initialization_gaps.clone();
        if let Some(gap) = xui18_preflight_gap {
            feature_local_gaps.push(gap);
        }
        if let Some(gap) = xui19_preflight_gap {
            feature_local_gaps.push(gap);
        }
        if let Some(gap) = xui22_preflight_gap {
            feature_local_gaps.push(gap);
        }
        if let Some(output) = decision_semantic_state
            .as_ref()
            .and_then(|surface| surface.pixel_output.as_ref())
        {
            let pending = self
                .xui19_pending
                .as_mut()
                .ok_or("XUI19 pixels were composed without a staged passive carrier")?;
            if pending.country_tag != output.country_tag
                || pending.source_sequence != output.source_sequence
                || pending.record_sha256 != output.record_sha256
                || pending.previous_record_sha256 != output.previous_record_sha256
                || pending.content_manifest_sha256 != output.content_manifest_sha256
            {
                return Err("XUI19 composed pixels disagree with the staged carrier".into());
            }
            viewer.decision_scroll_px = output.effective_scroll_px;
            pending.panel_drawn = true;
            pending.panel_rect = Some(output.panel_rect);
            pending.decision_grid_rect = Some(output.decision_grid_rect);
            pending.max_scroll_px = output.max_scroll_px;
            pending.category_extents = output.category_extents.clone();
            pending.card_hit_targets = output.card_hit_targets.clone();
            pending.custom_icon_hit_targets = output.custom_icon_hit_targets.clone();
            pending.provider_receipts = output.provider_receipts.clone();
            pending.xui22_widget_hit_targets = output.xui22_widget_hit_targets.clone();
        }
        let research_country = viewer.selected_country.clone();
        let mut research_available = false;
        let mut research_input_region = None;
        let mut research_node_hit_targets = Vec::new();
        let xui09_candidate = viewer
            .selected_country
            .is_selected()
            .then(|| viewer.selected_country.named_in(world_frame))
            .and_then(|named| {
            let Some(country) = named else {
                feature_local_gaps.push(
                    "XUI09:field=selected_country:domain=contention_census:unavailable=SELECTED_COUNTRY_IS_A_CONTENDED_KEY"
                        .to_string(),
                );
                return None;
            };
            let available_folders = match self
                .renderer
                .xui09_available_folders(self.static_research_catalog.as_ref())
            {
                Ok(folders) => folders,
                Err(error) => {
                    feature_local_gaps.push(format!(
                        "XUI09:field=effective_authored_content:domain=countrytechtreeview_and_countrytechnologyview:unavailable={error:?}"
                    ));
                    return None;
                }
            };
            let model = match game_ui::research_observer::observer_research_from_complete_frame(
                world_frame,
                country,
                self.static_research_catalog.as_ref(),
            ) {
                Ok(model) => model,
                Err(error) => {
                    feature_local_gaps.push(format!(
                        "XUI09:field=passive_all_country_research:domain=d18_g4_x9s1:unavailable={error}"
                    ));
                    return None;
                }
            };
            self.renderer
                .record_localisation_misses(model.localisation_misses.iter().cloned());
            Some(Xui09PresentedFrame {
                country_tag: country.to_string(),
                frame_hour: model.world_hour,
                available_folders,
                input_region: None,
                node_hit_targets: Vec::new(),
                model,
            })
        });
        if let Some(candidate) = xui09_candidate.as_ref() {
            research_available = true;
            if viewer.research.panel_open {
                let selected = viewer.research.selected_folder.as_deref();
                if selected
                    .is_none_or(|folder| !candidate.available_folders.iter().any(|v| v == folder))
                {
                    viewer.research.selected_folder = candidate.available_folders.first().cloned();
                    viewer.research.pan_pixels = [0, 0];
                    viewer.research.selected_technology_id = None;
                }
            }
        } else {
            viewer.research.reset_for_seek_or_country_change();
            self.xui09_drag_cursor = None;
            self.xui09_cursor = None;
        }
        self.xui09_pending_staged = true;
        self.xui09_pending = xui09_candidate;
        let mut peace_conference_available = false;
        let mut peace_conference_input_region = None;
        let xui08_candidate = world_frame
            .peace_conference_presentation_exact()
            .presentation
            .as_ref()
            .and_then(|carrier| {
                let continuity_ok = self.xui08_history_anchor.as_ref().is_none_or(|prior| {
                    prior.accepts(
                        carrier.frame_hour,
                        carrier.capture_sequence,
                        carrier.previous_record_sha256.as_deref(),
                        &carrier.record_sha256,
                    )
                });
                if !continuity_ok {
                    feature_local_gaps.push(
                        "XUI08:field=history:domain=_r2_peace_conference_presentation_exact:unavailable=NON_CONTIGUOUS_OR_REWRITTEN_PASSIVE_RECORD"
                            .into(),
                    );
                    return None;
                }
                let snapshot = match game_ui::readonly_design_peace::peace_conference_snapshot_from_world(world_frame) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        feature_local_gaps.push(format!(
                            "XUI08:field=semantic_snapshot:domain=_r2_peace_conference_presentation_exact:unavailable={error:?}"
                        ));
                        return None;
                    }
                };
                let admitted = match game_ui::readonly_design_peace::admit_peace_conference(&snapshot) {
                    Ok(admitted) => admitted,
                    Err(error) => {
                        feature_local_gaps.push(format!(
                            "XUI08:field=semantic_admission:domain=_r2_peace_conference_presentation_exact:unavailable={error:?}"
                        ));
                        return None;
                    }
                };
                Some(Xui08PresentedFrame {
                    frame_hour: carrier.frame_hour,
                    capture_sequence: carrier.capture_sequence,
                    record_sha256: carrier.record_sha256.clone(),
                    admitted,
                    input_region: None,
                    hit_targets: Vec::new(),
                    tooltip_targets: Vec::new(),
                })
            });
        reconcile_xui08_candidate(&mut viewer.peace_conference, xui08_candidate.as_ref());
        if xui08_candidate.is_none() {
            feature_local_gaps.push(format!(
                "XUI08:field=passive_global_presentation:domain=_r2_peace_conference_presentation_exact:unavailable={:?}",
                world_frame.peace_conference_presentation_exact_status()
            ));
        }
        if xui08_candidate.is_some() && viewer.peace_conference.open {
            if let Some(input) = self
                .xui08_presented
                .as_ref()
                .and_then(|presented| presented.input_region.clone())
            {
                viewer.peace_conference.advance_smooth_scroll(&input)?;
            }
        }
        self.xui08_pending_staged = true;
        self.xui08_pending = xui08_candidate;
        let politics_open = viewer.politics.panel_open;
        let politics_country = viewer.politics.country_tag.clone();
        let mut politics_available = false;
        let mut politics_input_region = None;
        let power_balance_open = viewer.power_balance.panel_open;
        let mut power_balance_available = self
            .xui18_pending
            .as_ref()
            .is_some_and(Xui18PresentedFrame::has_active_balance);
        let mut power_balance_panel_region = None;
        let mut combat_available = false;
        let mut combat_input_region = None;
        let xui11_candidate = xui11_passive_candidate(
            world_frame,
            viewer,
            self.xui11_history_anchor.as_ref(),
            [gfx.width as usize, gfx.height as usize],
            &mut feature_local_gaps,
        );
        if xui11_candidate.is_none() {
            feature_local_gaps.push(format!(
                "XUI11:field=passive_all_country_presentation:domain=_r2_politics_presentation_exact:unavailable={:?}",
                world_frame.politics_presentation_exact_status()
            ));
        }
        self.xui11_pending_staged = true;
        self.xui11_pending = xui11_candidate;
        let xui14_candidate = world_frame
            .intelligence_presentation_exact()
            .presentation
            .as_ref()
            .and_then(|carrier| {
                let continuity_ok = self
                    .xui14_history_anchor
                    .as_ref()
                    .is_none_or(|prior| {
                        prior.accepts(
                            carrier.frame_hour,
                            carrier.capture_sequence,
                            carrier.previous_record_sha256.as_deref(),
                            &carrier.record_sha256,
                        )
                    });
                if !continuity_ok {
                    feature_local_gaps.push(
                        "XUI14:field=history:domain=_r2_intelligence_presentation_exact:unavailable=NON_CONTIGUOUS_OR_REWRITTEN_PASSIVE_RECORD"
                            .into(),
                    );
                    return None;
                }
                Some(Xui14PresentedFrame {
                    frame_hour: carrier.frame_hour,
                    capture_sequence: carrier.capture_sequence,
                    record_sha256: carrier.record_sha256.clone(),
                    country_tags: carrier
                        .rows
                        .iter()
                        .map(|row| row.country_tag.clone())
                        .collect(),
                    input_region: None,
                    tooltip_targets: Vec::new(),
                })
            });
        if xui14_candidate.is_none() {
            feature_local_gaps.push(format!(
                "XUI14:field=passive_all_country_presentation:domain=_r2_intelligence_presentation_exact:unavailable={:?}",
                world_frame.intelligence_presentation_exact_status()
            ));
        }
        self.xui14_pending_staged = true;
        self.xui14_pending = xui14_candidate;
        feature_local_gaps.extend(xui20_preflight_gaps);
        let combat_open = viewer.combat.panel.is_some();
        let combat_panel = viewer.combat.panel.clone();
        let mut authored_roots: Vec<String> = authored_source_windows
            .iter()
            .map(|source| source.window_name.clone())
            .collect();
        let mut occupation_applied_scroll_row = None;
        let mut occupation_input_region = None;
        let mut character_applied_scroll_row = None;
        let mut character_input_region = None;
        let mut character_hit_targets = Vec::new();
        let mut character_tooltip_targets = Vec::new();
        let mut character_semantic_state = None;
        // Discarded: the join's product is the semantic receipt it pushes.
        let _ = match (
            viewer.selected_country.named_in(world_frame).map(|tag| tag.tag()),
            occupation_outcome.and_then(|outcome| outcome.surface.as_ref()),
        ) {
            (Some(country), Some(surface)) => {
                match game_ui::occupation_binding::occupation_alert_product_binding(
                    world_frame,
                    country,
                    surface,
                ) {
                    Ok(binding) => {
                        if let Some(binding) = binding.as_ref() {
                            semantic_receipts.push(format!(
                                "XUI16_ALERT:country={}:slot={}:source_ordinal={}:stable_id={}",
                                country,
                                binding.alert.registry_slot,
                                binding.icon.source_ordinal,
                                binding.alert.stable_id,
                            ));
                        } else {
                            semantic_receipts
                                .push(format!("XUI16_ALERT:country={country}:measured_empty"));
                        }
                        binding
                    }
                    Err(error) => {
                        feature_local_gaps.push(format!(
                            "XUI16_ALERT:field=d79_alert_join:domain=_r2_occupation_garrison_alert_live:unavailable={error:?}"
                        ));
                        None
                    }
                }
            }
            _ => None,
        };
        let mut production_lines_composited = false;
        let mut production_lines_applied_scroll_offset = None;
        let mut authored_panel_draw_count = 0usize;
        let authored_window_snapshot = None;
        let mut framebuffer = game_ui::font::blank_image(gfx.width as usize, gfx.height as usize);
        if viewer.decision_panel_open {
            if let Some(output) = decision_semantic_state
                .as_ref()
                .and_then(|surface| surface.pixel_output.as_ref())
            {
                game_ui::designer_observer::blend(&mut framebuffer, &output.image)?;
                authored_roots.push(game_ui::decision_chrome::DECISION_VIEW_GUI.to_string());
                authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                semantic_receipts.push(format!(
                    "XUI19:country={}:sequence={}:categories={}:cards={}:custom_icons={}:scroll={}/{}:manager-overlay-composited",
                    output.country_tag,
                    output.source_sequence,
                    output.category_extents.len(),
                    output.card_hit_targets.len(),
                    output.custom_icon_hit_targets.len(),
                    output.effective_scroll_px,
                    output.max_scroll_px,
                ));
            } else {
                feature_local_gaps.push(
                    "XUI19:field=pixel_surface:domain=strict_passive_decision_compositor:unavailable=OPEN_PANEL_PRODUCED_NO_PIXELS"
                        .into(),
                );
            }
        } else if let Some(pending) = self.xui19_pending.as_ref() {
            semantic_receipts.push(format!(
                "XUI19:country={}:sequence={}:viewer-closed:available",
                pending.country_tag, pending.source_sequence
            ));
        }
        if viewer.research.panel_open {
            'xui09: {
            if !viewer.selected_country.is_selected() {
                return Err("XUI09 research panel is open without an observer-selected country".into());
            }
            // A contended selection is DATA, not a broken viewer: it costs this
            // surface and names why, exactly where the renderer's own refusal
            // used to land. Escalating it would cost the whole frame's UI.
            let Some(selected_country) = viewer.selected_country.named_in(world_frame)
            else {
                feature_local_gaps.push(format!(
                    "XUI09:field=selected_country:domain=contention_census:unavailable=CONTENDED_KEY ({})",
                    viewer
                        .selected_country
                        .refusal_detail_in(world_frame)
                        .unwrap_or_default()
                ));
                break 'xui09;
            };
            if self.xui09_pending.is_some() {
                match self.renderer.compose_xui09_research_surface(
                    world_frame,
                    selected_country,
                    self.static_research_catalog.as_ref(),
                    &viewer.research,
                    gfx.width as usize,
                    gfx.height as usize,
                ) {
                    Ok(Some(output)) => {
                        let research_slot_count = output.model.research_slot_count;
                        game_ui::designer_observer::blend(&mut framebuffer, &output.image)?;
                        research_input_region = Some(output.input_region);
                        research_node_hit_targets = output.node_hit_targets.clone();
                        if let Some(pending) = self.xui09_pending.as_mut() {
                            pending.input_region = research_input_region;
                            pending.node_hit_targets = output.node_hit_targets;
                            pending.model = output.model;
                        }
                        research_available = true;
                        authored_roots
                            .push(game_ui::research_compositor::XUI09_GUI_SOURCE.to_string());
                        authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                        semantic_receipts.push(format!(
                            "XUI09:country={selected_country}:folder={}:nodes={}:links={}:xor={}:slots={}:manager-overlay-composited",
                            output.folder,
                            output.node_count,
                            output.line_cell_count,
                            output.xor_cell_count,
                            research_slot_count,
                        ));
                    }
                    Ok(None) => {
                        close_xui09_after_composition_failure(
                            &mut viewer.research,
                            &mut self.xui09_drag_cursor,
                            &mut self.xui09_cursor,
                        );
                        feature_local_gaps.push(
                            "XUI09:field=pixel_surface:domain=authenticated_xui09_compositor:unavailable=OPEN_PANEL_PRODUCED_NO_PIXELS"
                                .into(),
                        );
                    }
                    Err(error) => {
                        close_xui09_after_composition_failure(
                            &mut viewer.research,
                            &mut self.xui09_drag_cursor,
                            &mut self.xui09_cursor,
                        );
                        feature_local_gaps.push(format!(
                            "XUI09:field=effective_content_or_compositor_capability:domain=passive_all_country_research:unavailable={error:?}"
                        ));
                    }
                }
            } else {
                feature_local_gaps.push(
                    "XUI09:field=pixel_surface:domain=authenticated_xui09_compositor:unavailable=ADMISSION_REJECTED_CANDIDATE"
                        .into(),
                );
            }
            }
        } else if let Some(pending) = self.xui09_pending.as_ref() {
            semantic_receipts.push(format!(
                "XUI09:country={}:nodes={}:folders={}:viewer-closed:available",
                pending.country_tag,
                pending.model.nodes.len(),
                pending.available_folders.len(),
            ));
        }
        self.present_transaction.xui15_pending = None;
        if self.xui05_pending_owner.is_some() || self.xui05_pending_controller_checkpoint.is_some()
        {
            return Err("XUI05 previous staged frame has no Present disposition".into());
        }
        if let Some(authored) = self.authored_observer_ui.as_mut() {
            if let Some(canvas) = semantic
                .production_lines
                .as_ref()
                .and_then(|surface| surface.canvas.as_ref())
            {
                match authored.set_production_lines_canvas(
                    &self.content_admission,
                    gfx.width as usize,
                    gfx.height as usize,
                    canvas,
                    viewer_clock_micros,
                ) {
                    Ok(_) => production_lines_composited = true,
                    Err(error) => {
                        authored.reject_production_lines_canvas_feature_locally();
                        production_lines_available = false;
                        let gap = game_ui::production_renderer::ObserverUiGap {
                            surface: "XUI12/production_lines",
                            field: "effective_content_or_compositor_capability",
                            required_domain:
                                "effective authored root, templates, scrollbar style, and authenticated compositor capability",
                            reason: format!("XUI12 feature-local admission rejected: {error}"),
                        };
                        feature_local_gaps.push(format!(
                            "XUI12:field={}:domain={}:unavailable={}",
                            gap.field, gap.required_domain, gap.reason
                        ));
                        output_gaps.push(gap);
                    }
                }
            } else {
                authored.close_production_lines_canvas()?;
            }
            authored.bind_persistent_hud_roots(
                &self.content_admission,
                gfx.width as usize,
                gfx.height as usize,
                &authored_source_windows,
                &semantic.hud_binding,
                semantic.theatre.clone(),
                semantic.theatre_mode,
                semantic.theatre_settings_land_id,
            )?;
            let (panels, late_feature_local_rejection) = authored
                .compose_open_panels_with_feature_local_retry(
                    &mut framebuffer,
                    &self.content_admission,
                    viewer_clock_micros,
                )?;
            self.execute_lifecycle.mark_authored_candidate();
            if let Some(rejection) = late_feature_local_rejection {
                if rejection.production_lines.is_some() {
                    production_lines_composited = false;
                    production_lines_available = false;
                }
                for (line, gap) in rejection.observer_ui_gaps() {
                    feature_local_gaps.push(line);
                    output_gaps.push(gap);
                }
            }
            if production_lines_composited {
                production_lines_applied_scroll_offset =
                    authored.production_lines_scroll_offset()?;
            }
            authored_panel_draw_count = panels.len();
            for panel in &panels {
                let Some(frontend) = panel.frontend.as_ref() else {
                    continue;
                };
                semantic_receipts.push(format!(
                    "XUI26:family={}:window={}:source={}:instance={}:frame={}:drawn={}:admitted_bindings={}",
                    frontend.family_id,
                    panel.window_id,
                    frontend.source_file,
                    panel.runtime_instance_token,
                    panel.frame_generation,
                    panel.drawn_elements,
                    frontend.admitted_bindings.len(),
                ));
                for (slot_id, reason_code) in &frontend.unavailable_bindings {
                    feature_local_gaps.push(format!(
                        "XUI26:family={}:window={}:slot={}:unavailable={}:instance={}:frame={}",
                        frontend.family_id,
                        panel.window_id,
                        slot_id,
                        reason_code,
                        panel.runtime_instance_token,
                        panel.frame_generation,
                    ));
                }
            }
        }
        let xui08_delta = self.xui08_frame_clock.observe(viewer_clock_micros);
        if viewer.peace_conference.needs_surface() {
            if self
                .xui08_pending
                .as_ref()
                .is_some_and(|pending| pending.admitted.active())
            {
                let composed = match xui08_delta {
                    Ok(delta) => compose_and_install_xui08_surface(
                        &self.renderer, world_frame,
                        [gfx.width as usize, gfx.height as usize], delta,
                        &mut viewer.peace_conference,
                        self.xui08_pending.as_mut().expect("active candidate checked above"),
                        &mut framebuffer,
                    ),
                    Err(error) => Err(Xui08SurfaceStageError::Renderer(
                        game_ui::production_renderer::ProductionUiRejection::Asset(error.to_string()),
                    )),
                };
                match composed {
                    Ok(installed) => {
                        peace_conference_input_region = installed.input_region;
                        peace_conference_available = true;
                        if installed.visible {
                            authored_roots.push(game_ui::peace_conference_ui::PEACE_CONFERENCE_ROOT.into());
                            authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                        }
                        semantic_receipts.push(installed.receipt);
                    }
                    Err(Xui08SurfaceStageError::Renderer(error)) => feature_local_gaps.push(format!(
                        "XUI08:field=effective_content_or_compositor_capability:domain=passive_global_peace_conference:unavailable={error:?}"
                    )),
                    Err(Xui08SurfaceStageError::Execution(error)) => return Err(error.into()),
                }
            } else {
                feature_local_gaps.push(
                    "XUI08:field=pixel_surface:domain=authenticated_xui08_compositor:unavailable=NO_ACTIVE_PRESENTATION"
                        .into(),
                );
            }
        } else if self
            .xui08_pending
            .as_ref()
            .is_some_and(|pending| pending.admitted.active())
        {
            peace_conference_available = true;
            semantic_receipts.push("XUI08:viewer-closed:active-conference-available".into());
        }
        if politics_open {
            'xui11: {
            if !viewer.selected_country.is_selected() {
                return Err("XUI11 politics panel is open without an observer-selected country".into());
            }
            // A contended selection is DATA, not a broken viewer: it costs this
            // surface and names why, exactly where the renderer's own refusal
            // used to land. Escalating it would cost the whole frame's UI.
            let Some(selected_country) = viewer.selected_country.named_in(world_frame)
            else {
                feature_local_gaps.push(format!(
                    "XUI11:field=selected_country:domain=contention_census:unavailable=CONTENDED_KEY ({})",
                    viewer
                        .selected_country
                        .refusal_detail_in(world_frame)
                        .unwrap_or_default()
                ));
                break 'xui11;
            };
            if !viewer.politics.country_tag.is(selected_country.tag()) {
                return Err("XUI11 viewer country differs from observer-selected country".into());
            }
            if self.xui11_pending.is_some() {
                let power_balance_launcher = if self
                    .xui18_pending
                    .as_ref()
                    .is_some_and(Xui18PresentedFrame::has_active_balance)
                {
                    let admitted = match (
                        self.renderer.power_balance_catalog(),
                        self.renderer.power_balance_decisions(),
                    ) {
                        (Ok(catalog), Ok(decisions)) => {
                            crate::power_balance_runtime::admit_country_power_balance_ui(
                                world_frame,
                                selected_country.tag(),
                                catalog,
                                decisions,
                                self.renderer.content_manifest_digest_sha256(),
                            )
                        }
                        (Err(error), _) | (_, Err(error)) => Err(format!(
                            "XUI18 effective authored launcher catalog is unavailable: {error}"
                        )),
                    };
                    match admitted {
                        Ok(game_ui::power_balance::AdmittedPowerBalanceScreen::Visible(mut rows))
                            if rows.len() == 1 => Some(rows.remove(0)),
                        Ok(game_ui::power_balance::AdmittedPowerBalanceScreen::Visible(rows)) => {
                            return Err(format!(
                                "XUI18 launcher admission produced {} rows",
                                rows.len()
                            )
                            .into())
                        }
                        Ok(game_ui::power_balance::AdmittedPowerBalanceScreen::HiddenNoActiveBalance) => {
                            return Err(
                                "XUI18 visible launcher candidate became measured-empty".into()
                            )
                        }
                        Err(error) => {
                            return Err(format!("XUI18 launcher admission failed: {error}").into())
                        }
                    }
                } else {
                    None
                };
                match self.renderer.compose_xui11_politics_surface(
                    world_frame,
                    selected_country,
                    self.static_research_catalog.as_ref(),
                    power_balance_launcher.as_ref(),
                    &viewer.politics,
                    gfx.width as usize,
                    gfx.height as usize,
                ) {
                    Ok(Some(output)) => {
                        game_ui::designer_observer::blend(&mut framebuffer, &output.image)?;
                        politics_input_region = Some(output.input_region);
                        if let Some(pending) = self.xui11_pending.as_mut() {
                            pending.input_region = politics_input_region;
                            pending.presentation = Some(output.presentation.clone());
                            pending.tooltip_targets = output.tooltip_targets.clone();
                        }
                        politics_available = true;
                        authored_roots.push(game_ui::politics_state::POLITICS_ROOT.into());
                        authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                        semantic_receipts.push(format!(
                            "XUI11:country={selected_country}:record={}:rows={}:scroll={}:manager-overlay-composited",
                            output.source_record_identity_sha256,
                            output.presentation.ideas.len(),
                            viewer.politics.displayed_scroll_fixed,
                        ));
                    }
                    Ok(None) => feature_local_gaps.push(
                        "XUI11:field=pixel_surface:domain=authenticated_xui11_compositor:unavailable=OPEN_PANEL_PRODUCED_NO_PIXELS"
                            .into(),
                    ),
                    Err(error) => feature_local_gaps.push(format!(
                        "XUI11:field=effective_content_or_compositor_capability:domain=passive_all_country_politics:unavailable={error:?}"
                    )),
                }
            } else {
                feature_local_gaps.push(
                    "XUI11:field=pixel_surface:domain=authenticated_xui11_compositor:unavailable=HISTORY_REJECTED_CANDIDATE"
                        .into(),
                );
            }
            }
        } else if let Some(pending) = self.xui11_pending.as_ref() {
            politics_available = true;
            semantic_receipts.push(format!(
                "XUI11:country={}:record={}:viewer-closed:available",
                pending.country_tag, pending.record_sha256,
            ));
        }
        if power_balance_open {
            'xui18: {
            if !viewer.selected_country.is_selected() {
                return Err("XUI18 panel is open without an observer-selected country".into());
            }
            // A contended selection is DATA, not a broken viewer: it costs this
            // surface and names why. Escalating it would cost the whole frame's
            // UI, which is the rule the sibling blocks already follow.
            let Some(selected_country) = viewer.selected_country.named_in(world_frame)
            else {
                feature_local_gaps.push(format!(
                    "XUI18:field=selected_country:domain=contention_census:unavailable=CONTENDED_KEY ({})",
                    viewer
                        .selected_country
                        .refusal_detail_in(world_frame)
                        .unwrap_or_default()
                ));
                break 'xui18;
            };
            let selected_country = selected_country.tag();
            let admitted = match (
                self.renderer.power_balance_catalog(),
                self.renderer.power_balance_decisions(),
            ) {
                (Ok(catalog), Ok(decisions)) => {
                    crate::power_balance_runtime::admit_country_power_balance_ui(
                        world_frame,
                        selected_country,
                        catalog,
                        decisions,
                        self.renderer.content_manifest_digest_sha256(),
                    )
                }
                (Err(error), _) | (_, Err(error)) => Err(format!(
                    "XUI18 effective authored catalog is unavailable: {error}"
                )),
            };
            match admitted {
                Ok(game_ui::power_balance::AdmittedPowerBalanceScreen::Visible(panels)) => {
                    let [panel] = panels.as_slice() else {
                        return Err("XUI18 admitted visible set is not exactly one row".into());
                    };
                    match self.renderer.compose_xui18_power_balance_surface(
                        selected_country,
                        panel,
                        &viewer.power_balance,
                        gfx.width as usize,
                        gfx.height as usize,
                    ) {
                        Ok(Some(output)) => {
                            game_ui::designer_observer::blend(&mut framebuffer, &output.image)?;
                            power_balance_panel_region = Some(output.panel_region);
                            if let Some(pending) = self.xui18_pending.as_mut() {
                                pending.panel_region = Some(output.panel_region);
                                pending.close_region = Some(output.close_region);
                                pending.tooltip_targets = output.tooltip_targets;
                            }
                            power_balance_available = true;
                            authored_roots.push(output.authored_root);
                            authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                            semantic_receipts.push(format!(
                                "XUI18:country={selected_country}:record={}:decisions={}:receipt={}:manager-overlay-composited",
                                output.source_record_id,
                                output.decision_record_ids.len(),
                                panel.provenance.presentation_semantic_receipt_sha256,
                            ));
                        }
                        Ok(None) => {
                            return Err(
                                "XUI18 authenticated open panel produced no pixel surface".into()
                            );
                        }
                        Err(error) => {
                            return Err(format!(
                                "XUI18 authenticated compositor rejected the exact surface: {error:?}"
                            )
                            .into());
                        }
                    }
                }
                Ok(game_ui::power_balance::AdmittedPowerBalanceScreen::HiddenNoActiveBalance) => {
                    return Err(
                        "XUI18 authenticated panel remained open for measured-empty state".into(),
                    );
                }
                Err(error) => {
                    return Err(format!(
                        "XUI18 authenticated open panel failed exact admission: {error}"
                    )
                    .into());
                }
            }
            }
        } else if let Some(pending) = self.xui18_pending.as_ref() {
            semantic_receipts.push(match pending.availability {
                Xui18PresentedAvailability::Visible => format!(
                    "XUI18:country={}:receipt={}:viewer-closed:available",
                    pending.country_tag, pending.state_semantic_receipt_sha256,
                ),
                Xui18PresentedAvailability::MeasuredEmpty => format!(
                    "XUI18:country={}:receipt={}:viewer-closed:measured-empty",
                    pending.country_tag, pending.state_semantic_receipt_sha256,
                ),
            });
        }
        if viewer.intelligence.panel_open {
            'xui14: {
            if !viewer.selected_country.is_selected() {
                return Err("XUI14 intelligence panel is open without an observer-selected country".into());
            }
            // A contended selection is DATA, not a broken viewer: it costs this
            // surface and names why, exactly where the renderer's own refusal
            // used to land. Escalating it would cost the whole frame's UI.
            let Some(selected_country) = viewer.selected_country.named_in(world_frame)
            else {
                feature_local_gaps.push(format!(
                    "XUI14:field=selected_country:domain=contention_census:unavailable=CONTENDED_KEY ({})",
                    viewer
                        .selected_country
                        .refusal_detail_in(world_frame)
                        .unwrap_or_default()
                ));
                break 'xui14;
            };
            if !viewer.intelligence.country_tag.is(selected_country.tag()) {
                return Err("XUI14 viewer country differs from observer-selected country".into());
            }
            if self.xui14_pending.is_some() {
                match self.renderer.compose_xui14_intelligence_surface(
                    world_frame,
                    selected_country,
                    &viewer.intelligence,
                    gfx.width as usize,
                    gfx.height as usize,
                ) {
                    Ok(Some(output)) => {
                        game_ui::designer_observer::blend(&mut framebuffer, &output.image)?;
                        if let Some(pending) = self.xui14_pending.as_mut() {
                            pending.input_region = Some(output.input_region);
                            pending.tooltip_targets = output.tooltip_targets.clone();
                        }
                        authored_roots.push(output.authored_root.clone());
                        authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                        semantic_receipts.push(format!(
                            "XUI14:country={selected_country}:record={}:source={}:rows={}:surface={:?}:manager-overlay-composited",
                            output.record_sha256,
                            output.source_record_identity_sha256,
                            output.displayed_row_ids.len(),
                            viewer.intelligence.surface,
                        ));
                    }
                    Ok(None) => feature_local_gaps.push(
                        "XUI14:field=pixel_surface:domain=authenticated_xui14_compositor:unavailable=OPEN_PANEL_PRODUCED_NO_PIXELS"
                            .into(),
                    ),
                    Err(error) => feature_local_gaps.push(format!(
                        "XUI14:field=effective_content_or_compositor_capability:domain=passive_all_country_intelligence:unavailable={error:?}"
                    )),
                }
            } else {
                feature_local_gaps.push(
                    "XUI14:field=pixel_surface:domain=authenticated_xui14_compositor:unavailable=HISTORY_REJECTED_CANDIDATE"
                        .into(),
                );
            }
            }
        } else if let Some(pending) = self.xui14_pending.as_ref() {
            semantic_receipts.push(format!(
                "XUI14:record={}:countries={}:viewer-closed:available",
                pending.record_sha256,
                pending.country_tags.len(),
            ));
        }
        if combat_open {
            if self.xui20_pending.is_some() {
                match self.renderer.compose_xui20_combat_surface(
                    world_frame,
                    &viewer.combat,
                    gfx.width as usize,
                    gfx.height as usize,
                ) {
                    Ok(Some(output)) => {
                        game_ui::designer_observer::blend(&mut framebuffer, &output.image)?;
                        combat_input_region = Some(output.input_region);
                        if let Some(pending) = self.xui20_pending.as_mut() {
                            pending.input_region = combat_input_region;
                            pending.tooltip_targets = output.tooltip_targets.clone();
                        }
                        combat_available = true;
                        authored_roots.push(output.route.root.name.clone());
                        authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                        semantic_receipts.push(format!(
                            "XUI20:panel={:?}:record={}:rows={}:primary_scroll={}:secondary_scroll={}:manager-overlay-composited",
                            output.route.panel,
                            output.route.record_sha256,
                            output.route.presentation.dynamic_rows.len(),
                            viewer.combat.primary_scroll_px,
                            viewer.combat.secondary_scroll_px,
                        ));
                    }
                    Ok(None) => feature_local_gaps.push(
                        "XUI20:field=pixel_surface:domain=authenticated_xui20_compositor:unavailable=OPEN_PANEL_PRODUCED_NO_PIXELS"
                            .into(),
                    ),
                    Err(error) => feature_local_gaps.push(format!(
                        "XUI20:field=effective_content_or_compositor_capability:domain=passive_global_combat_presentation:unavailable={error:?}"
                    )),
                }
            } else {
                feature_local_gaps.push(
                    "XUI20:field=pixel_surface:domain=authenticated_xui20_compositor:unavailable=HISTORY_REJECTED_CANDIDATE"
                        .into(),
                );
            }
        } else if let Some(pending) = self.xui20_pending.as_ref() {
            combat_available = true;
            semantic_receipts.push(format!(
                "XUI20:record={}:panels={}:viewer-closed:available",
                pending.record_sha256,
                pending.available_panels.len(),
            ));
        }
        if viewer.occupation.panel != game_ui::occupation_compositor::OccupationPanel::Closed {
            'xui16: {
            if !viewer.selected_country.is_selected() {
                return Err("XUI16 panel is open without an observer-selected country".into());
            }
            // A contended selection is DATA, not a broken viewer: it costs this
            // surface and names why. Escalating it would cost the whole frame's
            // UI, which is the rule the sibling blocks already follow.
            let Some(selected_country) = viewer.selected_country.named_in(world_frame)
            else {
                feature_local_gaps.push(format!(
                    "XUI16:field=selected_country:domain=contention_census:unavailable=CONTENDED_KEY ({})",
                    viewer
                        .selected_country
                        .refusal_detail_in(world_frame)
                        .unwrap_or_default()
                ));
                break 'xui16;
            };
            let selected_country = selected_country.tag();
            if viewer.occupation.selected_country != selected_country {
                return Err(format!(
                    "XUI16 viewer country {:?} differs from observer-selected country {selected_country:?}",
                    viewer.occupation.selected_country
                )
                .into());
            }
            match occupation_outcome {
                None => feature_local_gaps.push(
                    "XUI16:field=observer_outcome:domain=_r2_occupation_ui_supplement_live:unavailable=XUI16_OBSERVER_OUTCOME_ABSENT"
                        .into(),
                ),
                Some(outcome) if outcome.unavailable_reason.is_some() => {
                    feature_local_gaps.push(format!(
                        "XUI16:field=occupation_surface:domain=_r2_occupation_ui_supplement_live:unavailable={}",
                        outcome
                            .unavailable_reason
                            .as_deref()
                            .expect("matched unavailable occupation outcome")
                    ));
                }
                Some(outcome) => match outcome.surface.as_ref() {
                    None => feature_local_gaps.push(
                        "XUI16:field=occupation_surface:domain=_r2_occupation_ui_supplement_live:unavailable=XUI16_ADMITTED_SURFACE_ABSENT"
                            .into(),
                    ),
                    Some(surface) => {
                        let plan = plan_xui16_panel(
                            viewer,
                            None,
                            Some(surface.source_record_id()),
                        )?
                        .ok_or("XUI16 open panel produced no render plan")?;
                        match self.renderer.compose_xui16_frame(
                            world_frame,
                            surface,
                            &viewer.occupation,
                            gfx.width as usize,
                            gfx.height as usize,
                        ) {
                            Ok(Some(output)) => {
                                if output.source_record_id != plan.source_record_id {
                                    return Err(format!(
                                        "XUI16 composed source {:?} differs from planned source {:?}",
                                        output.source_record_id, plan.source_record_id
                                    )
                                    .into());
                                }
                                game_ui::designer_observer::blend(
                                    &mut framebuffer,
                                    &output.image,
                                )?;
                                authored_roots.push(output.authored_root.clone());
                                authored_panel_draw_count += 1;
                                occupation_applied_scroll_row = Some(output.applied_scroll_row);
                                occupation_input_region = Some(output.input_region);
                                semantic_receipts.push(format!(
                                    "XUI16:country={}:panel={:?}:root={}:source={}:scroll_row={}:rows={:?}",
                                    plan.selected_country,
                                    plan.panel,
                                    output.authored_root,
                                    output.source_record_id,
                                    output.applied_scroll_row,
                                    output.materialized_row_ids,
                                ));
                            }
                            Ok(None) => feature_local_gaps.push(
                                "XUI16:field=pixel_surface:domain=authenticated_xui16_compositor:unavailable=XUI16_OPEN_PANEL_PRODUCED_NO_PIXELS"
                                    .into(),
                            ),
                            Err(error) => feature_local_gaps.push(format!(
                                "XUI16:field=pixel_surface:domain=authenticated_xui16_compositor:unavailable={error:?}"
                            )),
                        }
                    }
                },
            }
            }
        }
        if let Some(surface) = semantic.characters.as_ref() {
            if let Some(model) = surface.model.as_ref().filter(|_| surface.gaps.is_empty()) {
                if viewer.character_selection.panel == game_ui::character_ui::CharacterPanel::Closed
                {
                    character_semantic_state = Some(model.clone());
                    semantic_receipts.push(format!(
                        "XUI17:country={}:panel=Closed:semantic=admitted-presented",
                        model.selected_country
                    ));
                } else {
                    match self.renderer.compose_xui17_character_surface(
                        model,
                        &viewer.character_selection,
                        gfx.width as usize,
                        gfx.height as usize,
                    ) {
                        Ok(Some(output)) => {
                            game_ui::designer_observer::blend(&mut framebuffer, &output.image)?;
                            authored_roots.push(output.authored_root.clone());
                            authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                            character_applied_scroll_row = Some(output.applied_scroll_row);
                            character_input_region = Some(output.input_region);
                            character_hit_targets = output.hit_targets.clone();
                            character_tooltip_targets = output.tooltip_targets.clone();
                            character_semantic_state = Some(output.semantic_state.clone());
                            semantic_receipts.push(format!(
                                "XUI17:country={}:panel={:?}:root={}:scroll_row={}:rows={:?}",
                                model.selected_country,
                                viewer.character_selection.panel,
                                output.authored_root,
                                output.applied_scroll_row,
                                output.materialized_row_ids,
                            ));
                        }
                        Ok(None) => {
                            characters_available = false;
                            feature_local_gaps.push(
                                "XUI17:field=pixel_surface:domain=authenticated_xui17_compositor:unavailable=XUI17_OPEN_PANEL_PRODUCED_NO_PIXELS"
                                    .into(),
                            );
                        }
                        Err(error) => {
                            characters_available = false;
                            feature_local_gaps.push(format!(
                                "XUI17:field=pixel_surface:domain=authenticated_xui17_compositor:unavailable={error:?}"
                            ));
                        }
                    }
                }
            } else {
                characters_available = false;
                feature_local_gaps.extend(surface.gaps.iter().map(|gap| {
                    format!(
                        "XUI17:field={}:domain={}:unavailable={}",
                        gap.field, gap.required_domain, gap.reason
                    )
                }));
            }
        }
        if let Some(authored) = self.authored_observer_ui.as_ref() {
            semantic_receipts.push(format!(
                "XUI02:authored-window-manager:discovered={}:open={}:drawn={}:transition={}",
                authored.discovered_window_ids().count(),
                authored.manager().composed_stack().len(),
                authored_panel_draw_count,
                authored.manager().transition_ordinal(),
            ));
        }
        if let Some(surface) = semantic.designer.as_ref() {
            if let (Some(country_tag), Some(designer_viewer)) = (
                surface
                    .country_tag
                    .as_deref()
                    .and_then(|tag| world_frame.named_country(tag)),
                surface.viewer.as_ref(),
            ) {
                designer_available = apply_xui13_feature_local_composition(
                    self.renderer.compose_xui13_designer_surface(
                        world_frame,
                        country_tag,
                        designer_viewer,
                        gfx.width as usize,
                        gfx.height as usize,
                    ),
                    &mut framebuffer,
                    country_tag.tag(),
                    surface.measured_empty,
                    &mut authored_panel_draw_count,
                    &mut semantic_receipts,
                    &mut output_gaps,
                    &mut feature_local_gaps,
                )?;
            } else {
                feature_local_gaps.extend(surface.gaps.iter().map(|gap| {
                    format!(
                        "XUI13:field={}:domain={}:unavailable={}",
                        gap.field, gap.required_domain, gap.reason
                    )
                }));
            }
        }
        if let Some(surface) = semantic.trade.as_ref() {
            if let Some(panel) = surface.canvas.as_ref() {
                let composition = match xui15_smoothing_rejection.take() {
                    Some(error) => Err(error),
                    None => self.renderer.compose_xui15_trade_surface(
                        world_frame,
                        panel,
                        &viewer.trade,
                        gfx.width as usize,
                        gfx.height as usize,
                    ),
                };
                trade_available = apply_xui15_feature_local_composition(
                    composition,
                    &mut framebuffer,
                    panel,
                    &mut authored_panel_draw_count,
                    &mut semantic_receipts,
                    &mut output_gaps,
                    &mut feature_local_gaps,
                )?;
                if trade_available {
                    self.present_transaction
                        .stage_xui15(Xui15InteractionSurface::production(
                            panel.clone(),
                            [gfx.width as usize, gfx.height as usize],
                        ));
                } else {
                    trade_measured_empty = false;
                }
            } else {
                feature_local_gaps.extend(surface.gaps.iter().map(|gap| {
                    format!(
                        "XUI15:field={}:domain={}:unavailable={}",
                        gap.field, gap.required_domain, gap.reason
                    )
                }));
            }
        }
        if let Some(surface) = semantic.production_lines.as_ref() {
            if let Some(canvas) = surface
                .canvas
                .as_ref()
                .filter(|_| production_lines_composited)
            {
                semantic_receipts.push(format!(
                    "XUI12:country={}:rows={}:completeness={}:manager-composited",
                    canvas.country_tag,
                    canvas.rows.len(),
                    canvas.completeness_record,
                ));
            } else if let Some(route) = surface.route.as_ref() {
                semantic_receipts.push(format!(
                    "XUI12:country={}:military_rows={}:naval_rows={}:authored_root={}:available",
                    route.carrier.selected_country,
                    route.carrier.military_lines.len(),
                    route.carrier.naval_lines.len(),
                    route.authored_root.source_file,
                ));
            } else {
                feature_local_gaps.extend(surface.gaps.iter().map(|gap| {
                    format!(
                        "XUI12:field={}:domain={}:unavailable={}",
                        gap.field, gap.required_domain, gap.reason
                    )
                }));
            }
        }
        if focus_open {
            if focus_pixels_available {
                match self.renderer.compose_xui10_focus_surface(
                    &semantic.focus,
                    &viewer.focus,
                    gfx.width as usize,
                    gfx.height as usize,
                ) {
                    Ok(Some(surface)) => {
                        game_ui::designer_observer::blend(&mut framebuffer, &surface.output.image)?;
                        let scene = semantic
                            .focus
                            .scene()
                            .ok_or("XUI10 composed pixels have no authenticated prepared scene")?;
                        self.xui10_pending = Some(Xui10PresentedFrame {
                            country_tag: scene.country_tag().to_string(),
                            frame_hour: scene.frame_hour(),
                            input_region: surface.input_region,
                            node_regions: surface.node_regions.clone(),
                            progress: surface.progress.clone(),
                        });
                        authored_roots.push(game_ui::focus_observer::XUI10_GUI_ROOT.into());
                        authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                        semantic_receipts.push(format!(
                            "XUI10:country={}:tree={}:nodes={}:links={}:manager-overlay-composited",
                            semantic.focus.country_tag().unwrap_or(""),
                            semantic
                                .focus
                                .scene()
                                .map(|scene| scene.tree_id())
                                .unwrap_or(""),
                            semantic
                                .focus
                                .scene()
                                .map(|scene| scene.nodes().len())
                                .unwrap_or_default(),
                            semantic
                                .focus
                                .scene()
                                .map(|scene| scene.link_tiles().len())
                                .unwrap_or_default(),
                        ));
                        if let Some(progress) = surface.progress.as_ref() {
                            semantic_receipts.push(format!(
                                "XUI10:focus={}:display_progress_percent={}:presented-binding",
                                progress.focus_id, progress.display_progress_percent,
                            ));
                        }
                    }
                    Ok(None) => focus_pixels_available = false,
                    Err(error) => {
                        focus_pixels_available = false;
                        feature_local_gaps.push(format!(
                            "XUI10:field=effective_content_or_compositor_capability:domain=authenticated_evaluated_focus_surface:unavailable={error:?}"
                        ));
                    }
                }
            } else {
                feature_local_gaps.extend(semantic.focus.gaps().iter().map(|gap| {
                    format!(
                        "XUI10:field={}:domain={}:unavailable={}",
                        gap.field, gap.required_domain, gap.reason
                    )
                }));
            }
        } else if focus_semantic_available {
            semantic_receipts.push(format!(
                "XUI10:country={}:semantic-admitted:viewer-closed:evaluated-carrier={}",
                semantic.focus.country_tag().unwrap_or(""),
                semantic.focus.scene().is_some(),
            ));
        } else {
            feature_local_gaps.extend(semantic.focus.gaps().iter().map(|gap| {
                format!(
                    "XUI10:field={}:domain={}:unavailable={}",
                    gap.field, gap.required_domain, gap.reason
                )
            }));
        }
        let mut ability_button_regions = Vec::new();
        let mut ability_input_region = None;
        if abilities_open {
            if abilities_available {
                match self.renderer.compose_xui23_ability_surface(
                    &semantic.abilities,
                    &viewer.abilities,
                    ability_animation_elapsed_micros,
                    gfx.width as usize,
                    gfx.height as usize,
                ) {
                    Ok(Some(surface)) => {
                        game_ui::designer_observer::blend(&mut framebuffer, &surface.pixels.image)?;
                        authored_roots.push(surface.authored_root.to_string());
                        authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                        ability_input_region = Some(surface.input_region);
                        ability_button_regions = surface.button_regions.clone();
                        semantic_receipts.push(format!(
                            "XUI23:country={}:source={}:context={:?}:rows={}:root={}:manager-overlay-composited",
                            surface.country_tag,
                            semantic.abilities.source_record_id.as_deref().unwrap_or(""),
                            surface.selected_context,
                            surface.rows.len(),
                            surface.authored_root,
                        ));
                    }
                    Ok(None) => semantic_receipts.push(format!(
                        "XUI23:country={}:source={}:context={:?}:measured-empty",
                        semantic.abilities.country_tag.as_deref().unwrap_or(""),
                        semantic.abilities.source_record_id.as_deref().unwrap_or(""),
                        semantic.abilities.selected_context,
                    )),
                    Err(error) => {
                        abilities_available = false;
                        feature_local_gaps.push(format!(
                            "XUI23:field=effective_content_or_compositor_capability:domain=authenticated_passive_ability_surface:unavailable={error:?}"
                        ));
                    }
                }
            } else {
                feature_local_gaps.extend(semantic.abilities.gaps.iter().map(|gap| {
                    format!(
                        "XUI23:field={}:domain={}:unavailable={}",
                        gap.field, gap.required_domain, gap.reason
                    )
                }));
            }
        } else if abilities_available {
            semantic_receipts.push(format!(
                "XUI23:country={}:source={}:viewer-closed:available",
                semantic.abilities.country_tag.as_deref().unwrap_or(""),
                semantic.abilities.source_record_id.as_deref().unwrap_or(""),
            ));
        } else {
            feature_local_gaps.extend(semantic.abilities.gaps.iter().map(|gap| {
                format!(
                    "XUI23:field={}:domain={}:unavailable={}",
                    gap.field, gap.required_domain, gap.reason
                )
            }));
        }
        let event_history_available = semantic.events.admitted_history.is_some();
        let event_measured_empty = event_history_available && semantic.events.measured_empty;
        if event_history_available {
            viewer.events.focused_event_sequence = semantic.events.focused_sequence;
        }
        let mut event_pixels_available = event_measured_empty;
        let mut xui21_popups = Vec::new();
        if event_history_available && !event_measured_empty {
            match self.renderer.compose_xui21_event_stack(
                &semantic.events,
                gfx.width as usize,
                gfx.height as usize,
            ) {
                Ok(outputs) if outputs.len() == semantic.events.visible_sequences.len() => {
                    for output in outputs {
                        game_ui::designer_observer::blend(&mut framebuffer, &output.image)?;
                        xui21_popups.push(Xui21PresentedPopup {
                            event_sequence: output.event_sequence,
                            root_source: output.root_source,
                            root_window: output.root_window,
                            root_region: output.root_region,
                            minimize_region: output.minimize_region,
                            minimize_child_path: output.minimize_child_path,
                            minimize_tooltip_key: output.minimize_tooltip_key,
                            option_targets: output.option_targets,
                        });
                        authored_panel_draw_count =
                            authored_panel_draw_count.saturating_add(1);
                    }
                    event_pixels_available = true;
                    semantic_receipts.push(format!(
                        "XUI21:country={}:tick={}:visible={:?}:focused={:?}:authored-stack-composited",
                        semantic.events.country_tag.as_deref().unwrap_or(""),
                        semantic.events.event_tick.unwrap_or_default(),
                        semantic.events.visible_sequences,
                        semantic.events.focused_sequence,
                    ));
                }
                Ok(outputs) => feature_local_gaps.push(format!(
                    "XUI21:field=pixel_stack:domain=effective_event_chrome:unavailable=VISIBLE_COUNT_{}_OUTPUT_COUNT_{}",
                    semantic.events.visible_sequences.len(),
                    outputs.len()
                )),
                Err(error) => feature_local_gaps.push(format!(
                    "XUI21:field=effective_content_or_compositor:domain=authored_event_popup:unavailable={error:?}"
                )),
            }
        } else if event_measured_empty {
            semantic_receipts.push("XUI21:visible-popups:measured-empty".into());
        } else {
            feature_local_gaps.extend(semantic.events.gaps.iter().map(|gap| {
                format!(
                    "XUI21:field={}:domain={}:unavailable={}",
                    gap.field, gap.required_domain, gap.reason
                )
            }));
        }
        let xui21_candidate = xui21_staged_candidate(
            semantic.events.country_tag.as_ref(),
            semantic.events.event_tick,
            event_history_available,
            event_pixels_available,
            &semantic.events.visible_sequences,
            semantic.events.focused_sequence,
            xui21_popups,
        );
        stage_xui21_candidate(
            &self.xui21_pending_viewer_checkpoint,
            &mut self.xui21_pending,
            &mut self.xui21_pending_staged,
            xui21_candidate,
        )?;
        let xui09_cursor = self.xui09_cursor;
        let xui09_cursor_inside = viewer.research.panel_open
            && self
                .xui09_pending
                .as_ref()
                .and_then(|pending| pending.input_region)
                .zip(xui09_cursor)
                .is_some_and(|(region, (x, y))| region.contains(x, y));
        let xui09_payloads = self
            .xui09_pending
            .as_ref()
            .and_then(|pending| {
                xui09_inspection_target(
                    &pending.node_hit_targets,
                    viewer.research.selected_technology_id.as_deref(),
                    xui09_cursor,
                )
                .map(|target| (&pending.model, target))
            })
            .map(|(model, target)| self.renderer.xui09_tooltip_payloads(model, target))
            .transpose()
            .map_err(|error| format!("XUI09/XUI05 payload admission refused: {error:?}"))?;
        let xui08_cursor = self.xui08_cursor;
        let xui08_cursor_inside = viewer.peace_conference.open
            && self
                .xui08_pending
                .as_ref()
                .and_then(|pending| pending.input_region.clone())
                .zip(xui08_cursor)
                .is_some_and(|(region, point)| region.panel_contains(point));
        let xui08_payloads = self.xui08_pending.as_ref().and_then(|pending| {
            xui08_cursor.and_then(|point| {
                pending
                    .tooltip_targets
                    .iter()
                    .rev()
                    .find(|target| target.contains(point))
                    .map(|target| target.payloads.clone())
            })
        });
        let xui20_cursor = self.xui20_cursor;
        let xui20_cursor_inside = viewer.combat.panel.is_some()
            && self
                .xui20_pending
                .as_ref()
                .and_then(|pending| pending.input_region)
                .zip(xui20_cursor)
                .is_some_and(|(region, point)| region.panel.contains_half_open(point));
        let xui20_payload = self.xui20_pending.as_ref().and_then(|pending| {
            xui20_cursor.and_then(|point| {
                pending
                    .tooltip_targets
                    .iter()
                    .rev()
                    .find(|target| target.rect.contains_half_open(point))
                    .map(|target| target.payload.clone())
            })
        });
        let xui17_cursor = self.xui17_cursor;
        let xui17_cursor_inside = viewer.character_selection.panel
            != game_ui::character_ui::CharacterPanel::Closed
            && character_input_region
                .zip(xui17_cursor)
                .is_some_and(|(region, (x, y))| region.contains_panel_point(x, y));
        let xui17_payloads = xui17_cursor
            .and_then(|point| {
                character_tooltip_targets
                    .iter()
                    .rev()
                    .find(|target| target.contains(point))
            })
            .map(|target| self.renderer.xui17_tooltip_payloads(target))
            .transpose()
            .map_err(|error| format!("XUI17/XUI05 payload admission refused: {error:?}"))?
            .flatten();
        let xui14_cursor = self.xui14_cursor;
        let xui14_cursor_inside = viewer.intelligence.panel_open
            && self
                .xui14_pending
                .as_ref()
                .and_then(|pending| pending.input_region)
                .zip(xui14_cursor)
                .is_some_and(|(region, (x, y))| region.panel_contains(x, y));
        let xui14_payloads = self.xui14_pending.as_ref().and_then(|pending| {
            xui14_cursor.and_then(|point| {
                game_ui::intelligence_presentation::intelligence_tooltip_payloads_at(
                    &pending.tooltip_targets,
                    point,
                )
                .cloned()
            })
        });
        let xui18_cursor = self.xui18_cursor;
        let xui18_cursor_inside = viewer.power_balance.panel_open
            && self
                .xui18_pending
                .as_ref()
                .and_then(|pending| pending.panel_region)
                .zip(xui18_cursor)
                .is_some_and(|(region, (x, y))| region.contains(x, y));
        let xui18_payloads = self.xui18_pending.as_ref().and_then(|pending| {
            xui18_cursor.and_then(|point| {
                pending
                    .tooltip_targets
                    .iter()
                    .rev()
                    .find(|target| target.contains(point))
                    .map(|target| target.payloads.clone())
            })
        });
        let xui11_cursor = self.xui11_cursor;
        let xui11_cursor_inside = self
            .xui11_pending
            .as_ref()
            .and_then(|pending| pending.input_region)
            .zip(xui11_cursor)
            .is_some_and(|(region, (x, y))| region.panel_contains(x, y));
        let xui11_owner = self.xui11_pending.as_ref().and_then(|pending| {
            xui11_cursor.and_then(|point| {
                game_ui::politics_state::politics_tooltip_owner_at(&pending.tooltip_targets, point)
                    .cloned()
            })
        });
        let xui11_payloads = match (
            xui11_owner.as_ref(),
            self.xui11_pending
                .as_ref()
                .and_then(|pending| pending.presentation.as_ref()),
            world_frame
                .politics_presentation_exact()
                .presentation
                .as_ref(),
        ) {
            (Some(owner), Some(presentation), Some(carrier)) => self
                .renderer
                .xui11_tooltip_payloads(
                    presentation,
                    &viewer.politics,
                    owner,
                    &carrier.producer_manifest_sha256,
                    gfx.width as usize,
                    gfx.height as usize,
                )
                .map_err(|error| format!("XUI11/XUI05 payload admission refused: {error:?}"))?,
            _ => None,
        };
        let xui21_cursor = self.xui21_cursor;
        let xui21_cursor_inside =
            self.xui21_pending
                .as_ref()
                .zip(xui21_cursor)
                .is_some_and(|(pending, (x, y))| {
                    pending
                        .popups
                        .iter()
                        .rev()
                        .any(|popup| popup.root_region.contains(x, y))
                });
        let xui21_tooltip = self
            .xui21_pending
            .as_ref()
            .zip(xui21_cursor)
            .and_then(|(pending, (x, y))| {
                pending
                    .popups
                    .iter()
                    .rev()
                    .find(|popup| {
                        popup.minimize_tooltip_key.is_some() && popup.minimize_region.contains(x, y)
                    })
                    .map(|popup| (pending, popup))
            })
            .map(|(pending, popup)| {
                self.renderer.xui21_minimize_tooltip_payloads(
                    &pending.country_tag,
                    popup.event_sequence,
                    &popup.root_source,
                    &popup.root_window,
                    &popup.minimize_child_path,
                    popup
                        .minimize_tooltip_key
                        .as_deref()
                        .expect("filtered to authored tooltip key"),
                )
            })
            .transpose()
            .map_err(|error| format!("XUI21/XUI05 payload admission refused: {error:?}"))?;
        let xui19_cursor = self.xui19_cursor;
        let xui19_presented = xui19_matching_presented(viewer, self.xui19_presented.as_ref());
        let xui19_cursor_inside = viewer.decision_panel_open
            && xui19_presented
                .and_then(|presented| presented.panel_rect)
                .zip(xui19_cursor)
                .is_some_and(|(rect, (x, y))| xui19_rect_contains(rect, x, y));
        let xui19_payloads = match (xui19_presented, xui19_cursor) {
            (Some(presented), Some((x, y))) if viewer.decision_panel_open => {
                if let Some(target) =
                    presented
                        .xui22_widget_hit_targets
                        .iter()
                        .rev()
                        .find(|target| {
                            target.has_authored_tooltip && xui19_rect_contains(target.rect, x, y)
                        })
                {
                    self.renderer.xui22_widget_tooltip_payloads(
                        &presented.country_tag,
                        &presented.content_manifest_sha256,
                        target,
                    )
                } else if let Some(target) = presented
                    .custom_icon_hit_targets
                    .iter()
                    .rev()
                    .find(|target| xui19_rect_contains(target.rect, x, y))
                {
                    self.renderer.xui19_custom_icon_tooltip_payloads(
                        &presented.country_tag,
                        &presented.content_manifest_sha256,
                        target,
                    )
                } else if let Some(target) = presented
                    .card_hit_targets
                    .iter()
                    .rev()
                    .find(|target| xui19_rect_contains(target.rect, x, y))
                {
                    self.renderer.xui19_card_tooltip_payloads(
                        &presented.country_tag,
                        &presented.content_manifest_sha256,
                        target,
                    )
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
        .map_err(|error| format!("XUI19/XUI05 payload admission refused: {error:?}"))?;
        let tooltip_context = match self
            .authored_observer_ui
            .as_ref()
            .ok_or("XUI02 authored observer UI is unavailable")?
            .authenticated_pending_tooltip_context_at_cursor(&self.content_admission)
        {
            Ok(context) => context,
            Err(error) => {
                feature_local_gaps.push(format!(
                    "XUI05:field=authenticated_hover_context:domain=xui02_xui04_staged_geometry:unavailable={error}"
                ));
                None
            }
        };
        self.xui05_pending_controller_checkpoint = Some(self.xui05_controller.clone());
        self.execute_lifecycle.mark_late_candidates();
        let tooltip_disposition = if xui21_cursor_inside {
            let shell = self
                .authored_observer_ui
                .as_ref()
                .ok_or("XUI02 authored observer UI is unavailable")?
                .authenticated_tooltip_shell_surface(&self.content_admission)?;
            self.renderer.compose_packet_xui05_tooltip(
                &mut self.xui05_controller,
                &shell,
                xui21_tooltip.as_ref(),
                "XUI21",
                &viewer.selected_country,
                xui21_cursor.unwrap_or_default(),
                viewer_clock_micros,
                gfx.width as usize,
                gfx.height as usize,
            )
        } else if xui17_cursor_inside {
            let shell = self
                .authored_observer_ui
                .as_ref()
                .ok_or("XUI02 authored observer UI is unavailable")?
                .authenticated_tooltip_shell_surface(&self.content_admission)?;
            self.renderer.compose_packet_xui05_tooltip(
                &mut self.xui05_controller,
                &shell,
                xui17_payloads.as_ref(),
                "XUI17",
                &viewer.selected_country,
                xui17_cursor.unwrap_or_default(),
                viewer_clock_micros,
                gfx.width as usize,
                gfx.height as usize,
            )
        } else if xui19_cursor_inside {
            let shell = self
                .authored_observer_ui
                .as_ref()
                .ok_or("XUI02 authored observer UI is unavailable")?
                .authenticated_tooltip_shell_surface(&self.content_admission)?;
            self.renderer.compose_xui19_xui05_tooltip(
                &mut self.xui05_controller,
                &shell,
                xui19_payloads.as_ref(),
                &viewer.selected_country,
                xui19_cursor.unwrap_or_default(),
                viewer_clock_micros,
                gfx.width as usize,
                gfx.height as usize,
            )
        } else if xui09_cursor_inside {
            let shell = self
                .authored_observer_ui
                .as_ref()
                .ok_or("XUI02 authored observer UI is unavailable")?
                .authenticated_tooltip_shell_surface(&self.content_admission)?;
            self.renderer.compose_packet_xui05_tooltip(
                &mut self.xui05_controller,
                &shell,
                xui09_payloads.as_ref(),
                "XUI09",
                &viewer.selected_country,
                xui09_cursor.unwrap_or_default(),
                viewer_clock_micros,
                gfx.width as usize,
                gfx.height as usize,
            )
        } else if xui08_cursor_inside {
            let shell = self
                .authored_observer_ui
                .as_ref()
                .ok_or("XUI02 authored observer UI is unavailable")?
                .authenticated_tooltip_shell_surface(&self.content_admission)?;
            self.renderer.compose_packet_xui05_tooltip(
                &mut self.xui05_controller,
                &shell,
                xui08_payloads.as_ref(),
                "XUI08",
                &viewer.selected_country,
                xui08_cursor.unwrap_or_default(),
                viewer_clock_micros,
                gfx.width as usize,
                gfx.height as usize,
            )
        } else if xui20_cursor_inside {
            let shell = self
                .authored_observer_ui
                .as_ref()
                .ok_or("XUI02 authored observer UI is unavailable")?
                .authenticated_tooltip_shell_surface(&self.content_admission)?;
            self.renderer.compose_xui20_xui05_tooltip(
                &mut self.xui05_controller,
                &shell,
                xui20_payload.as_ref(),
                &viewer.selected_country,
                xui20_cursor.unwrap_or_default(),
                viewer_clock_micros,
                gfx.width as usize,
                gfx.height as usize,
            )
        } else if xui14_cursor_inside {
            let shell = self
                .authored_observer_ui
                .as_ref()
                .ok_or("XUI02 authored observer UI is unavailable")?
                .authenticated_tooltip_shell_surface(&self.content_admission)?;
            self.renderer.compose_packet_xui05_tooltip(
                &mut self.xui05_controller,
                &shell,
                xui14_payloads.as_ref(),
                "XUI14",
                &viewer.selected_country,
                xui14_cursor.unwrap_or_default(),
                viewer_clock_micros,
                gfx.width as usize,
                gfx.height as usize,
            )
        } else if xui18_cursor_inside {
            let shell = self
                .authored_observer_ui
                .as_ref()
                .ok_or("XUI02 authored observer UI is unavailable")?
                .authenticated_tooltip_shell_surface(&self.content_admission)?;
            self.renderer.compose_packet_xui05_tooltip(
                &mut self.xui05_controller,
                &shell,
                xui18_payloads.as_ref(),
                "XUI18",
                &viewer.selected_country,
                xui18_cursor.unwrap_or_default(),
                viewer_clock_micros,
                gfx.width as usize,
                gfx.height as usize,
            )
        } else if xui11_cursor_inside {
            let shell = self
                .authored_observer_ui
                .as_ref()
                .ok_or("XUI02 authored observer UI is unavailable")?
                .authenticated_tooltip_shell_surface(&self.content_admission)?;
            self.renderer.compose_packet_xui05_tooltip(
                &mut self.xui05_controller,
                &shell,
                xui11_payloads.as_ref(),
                "XUI11",
                &viewer.selected_country,
                xui11_cursor.unwrap_or_default(),
                viewer_clock_micros,
                gfx.width as usize,
                gfx.height as usize,
            )
        } else {
            self.renderer.compose_authenticated_xui05_tooltip(
                &mut self.xui05_controller,
                tooltip_context.as_ref(),
                &viewer.selected_country,
                viewer_clock_micros,
                gfx.width as usize,
                gfx.height as usize,
            )
        };
        match tooltip_disposition {
            Ok(game_ui::production_renderer::ProductionXui05Disposition::NoHoveredOwner) => {
                semantic_receipts.push("XUI05:no-hovered-owner".into());
            }
            Ok(
                game_ui::production_renderer::ProductionXui05Disposition::HoveredWithoutAuthoredTooltip,
            ) => {
                semantic_receipts.push("XUI05:hovered-without-authored-tooltip:measured-empty".into());
            }
            Ok(game_ui::production_renderer::ProductionXui05Disposition::DwellPending) => {
                semantic_receipts.push("XUI05:authenticated-hover:dwell-pending".into());
            }
            Ok(game_ui::production_renderer::ProductionXui05Disposition::Visible {
                image,
                owner,
                authenticated_owner,
                row_count,
                delayed_content_visible,
            }) => {
                if let Err(blend_error) =
                    game_ui::designer_observer::blend(&mut framebuffer, &image)
                {
                    self.reject_unpresented_authored_frame(viewer).map_err(|rollback_error| {
                        format!(
                            "XUI05 final-frame blend failed ({blend_error}); authenticated frame rollback also failed: {rollback_error}"
                        )
                    })?;
                    return Err(blend_error.into());
                }
                let instance_record = format!(
                    "XUI05:{}:{}:{:?}:{}",
                    owner.source_file(),
                    owner.root_window(),
                    owner.element_path(),
                    owner.engine_instance_token(),
                );
                self.xui05_pending_owner = Some((authenticated_owner, instance_record));
                authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                semantic_receipts.push(format!(
                    "XUI05:rows={row_count}:delayed={delayed_content_visible}:manager-overlay-composited-last"
                ));
            }
            Ok(game_ui::production_renderer::ProductionXui05Disposition::SemanticVisible {
                image,
                owner,
                row_count,
                delayed_content_visible,
            }) => {
                game_ui::designer_observer::blend(&mut framebuffer, &image)?;
                authored_panel_draw_count = authored_panel_draw_count.saturating_add(1);
                semantic_receipts.push(format!(
                    "XUI05:owner={:?}:rows={row_count}:delayed={delayed_content_visible}:semantic-overlay-composited-last",
                    owner.element_path(),
                ));
            }
            Err(error) => {
                feature_local_gaps.push(format!(
                    "XUI05:field=content_or_compositor_capability:domain=authenticated_authored_tooltip:unavailable={error:?}"
                ));
            }
        }
        self.xui23_pending = Some(Xui23PresentedFrame {
            country_tag: viewer.selected_country.clone(),
            frame_hour: world_frame.hour_index(),
            viewer_clock_micros,
            animation_elapsed_micros: ability_animation_elapsed_micros,
            contexts: semantic
                .abilities
                .admitted
                .as_ref()
                .map(|frame| frame.snapshot.contexts.clone())
                .unwrap_or_default(),
            input_region: ability_input_region,
            button_regions: ability_button_regions.clone(),
        });
        if let Err(upload_error) = self
            .image
            .set_rgba(gfx, &framebuffer)
            .and_then(|_| self.image.draw(gfx))
        {
            self.reject_unpresented_authored_frame(viewer)
                .map_err(|rollback_error| {
                    format!(
                        "observer UI upload/draw failed ({upload_error}); authenticated frame rollback also failed: {rollback_error}"
                    )
                })?;
            return Err(upload_error);
        }
        if viewer.selected_country.is_selected() {
            // ONE ASK for this whole block: what this hour's census says the
            // stored selection names, and the reason when it names nothing.
            let named = viewer.selected_country.named_in(world_frame);
            if let (Some(country), Some(decisions)) = (named, decision_semantic_state.as_ref())
            {
                semantic_receipts.push(format!(
                    "XUI19:country={}:sequence={:?}:record={:?}:templates={}:scene={}:pixels={}:gaps={}",
                    country.tag(),
                    decisions.source_sequence,
                    decisions.record_sha256,
                    decisions.authored_template_count,
                    decisions.authored_scene_source,
                    decisions.pixel_output.is_some(),
                    decisions.gaps.len(),
                ));
            }
            if named.is_none() {
                let detail = viewer
                    .selected_country
                    .refusal_detail_in(world_frame)
                    .unwrap_or_default();
                feature_local_gaps.push(format!(
                    "XUI11:field=selected_country:domain=contention_census:unavailable=CONTENDED_KEY ({detail})"
                ));
                feature_local_gaps.push(format!(
                    "XUI18:field=selected_country:domain=contention_census:unavailable=CONTENDED_KEY ({detail})"
                ));
                if self.scripted_gui_registry.is_some() {
                    feature_local_gaps.push(format!(
                        "XUI22:field=selected_country:domain=contention_census:unavailable=CONTENDED_KEY ({detail})"
                    ));
                }
            }
            if let Some(country) = named {
                match game_ui::observer_panels::admit_politics_observer_panel(
                    world_frame,
                    country,
                    self.static_research_catalog.as_ref(),
                ) {
                    Ok(panel) => semantic_receipts.push(format!(
                        "XUI11:country={country}:presentation={:?}",
                        panel.presentation
                    )),
                    Err(error) => feature_local_gaps.push(format!("XUI11:{error}")),
                }
                match game_ui::observer_panels::admit_power_balance_observer_panel(
                    world_frame,
                    country,
                ) {
                    Ok(panel) => semantic_receipts.push(format!(
                        "XUI18:country={country}:rows={}:presentation={:?}",
                        panel.applicable.len(),
                        panel.presentation
                    )),
                    Err(error) => feature_local_gaps.push(format!("XUI18:{error}")),
                }
                if let Some(registry) = self.scripted_gui_registry.as_ref() {
                    match game_ui::observer_panels::admit_scripted_gui_observer_panel(
                        registry,
                        world_frame,
                        country,
                    ) {
                        Ok(panel) => semantic_receipts.push(format!(
                            "XUI22:country={country}:rows={}:presentation={:?}",
                            panel.rows.len(),
                            panel.presentation
                        )),
                        Err(error) => feature_local_gaps.push(format!("XUI22:{error}")),
                    }
                }
            }
        }
        Ok(ObserverUiOutcome {
            localisation_misses: {
                let mut misses = self.renderer.take_localisation_misses();
                if let Some(surface) = occupation_outcome.and_then(|outcome| outcome.surface.as_ref())
                {
                    misses.absorb(surface.localisation_misses().clone());
                }
                misses
            },
            authored_roots,
            authored_panel_draw_count,
            authored_window_snapshot,
            selected_country: viewer.selected_country.clone(),
            trade_panel_open: viewer.trade_panel_open,
            decision_panel_open: viewer.decision_panel_open,
            trade_semantic_state,
            decision_semantic_state,
            focus_open,
            focus_country,
            focus_semantic_available,
            focus_pixels_available,
            focus_measured_empty,
            trade_available,
            trade_measured_empty,
            production_lines_open,
            production_lines_country,
            production_lines_available,
            production_lines_applied_scroll_offset,
            designer_open,
            designer_country,
            designer_available,
            designer_measured_empty,
            characters_open,
            characters_country,
            characters_available,
            character_applied_scroll_row,
            character_input_region,
            character_hit_targets,
            character_semantic_state,
            research_open: viewer.research.panel_open,
            research_country,
            research_available,
            research_folder: viewer.research.selected_folder.clone(),
            research_input_region,
            research_node_hit_targets,
            politics_open,
            politics_country,
            politics_available,
            politics_input_region,
            power_balance_open,
            power_balance_available,
            power_balance_panel_region,
            peace_conference_open: viewer.peace_conference.open,
            peace_conference_available,
            peace_conference_input_region,
            combat_open,
            combat_panel,
            combat_available,
            combat_input_region,
            semantic_receipts,
            feature_local_gaps,
            abilities_open,
            abilities_country,
            abilities_available,
            abilities_measured_empty,
            ability_button_regions,
            event_history_available,
            event_pixels_available,
            event_measured_empty,
            event_visible_sequences: semantic.events.visible_sequences.clone(),
            event_focused_sequence: semantic.events.focused_sequence,
            gaps: output_gaps,
            occupation_applied_scroll_row,
            occupation_input_region,
            promoted_extension_packets: game_ui::production_renderer::PROMOTED_EXTENSION_PACKETS
                .iter()
                .map(|packet| (*packet).to_string())
                .collect(),
        })
    }

    pub fn invalidate(&mut self) {
        self.image.clear();
        self.present_transaction.invalidate_runtime_surfaces();
        self.xui23_presented = None;
        self.xui23_pending = None;
        self.xui10_presented = None;
        self.xui10_pending = None;
        self.xui10_drag_cursor = None;
        self.xui09_presented = None;
        self.xui09_pending = None;
        self.xui09_pending_staged = false;
        self.xui09_pending_viewer_checkpoint = None;
        self.xui09_drag_cursor = None;
        self.xui09_cursor = None;
        self.xui11_presented = None;
        self.xui11_pending = None;
        self.xui11_pending_staged = false;
        self.xui11_pending_viewer_checkpoint = None;
        self.xui11_cursor = None;
        self.xui18_presented = None;
        self.xui18_pending = None;
        self.xui18_pending_staged = false;
        self.xui18_pending_viewer_checkpoint = None;
        self.xui18_cursor = None;
        self.xui19_presented = None;
        self.xui19_history_anchor = None;
        self.xui19_pending = None;
        self.xui19_pending_staged = false;
        self.xui19_pending_viewer_checkpoint = None;
        self.xui19_cursor = None;
        self.xui08_presented = None;
        self.xui08_pending = None;
        self.xui08_pending_staged = false;
        self.xui08_pending_viewer_checkpoint = None;
        self.xui08_pending_viewer_input = None;
        self.xui08_cursor = None;
        self.xui14_presented = None;
        self.xui14_history_anchor = None;
        self.xui14_pending = None;
        self.xui14_pending_staged = false;
        self.xui14_pending_viewer_checkpoint = None;
        self.xui14_cursor = None;
        self.xui17_cursor = None;
        self.xui20_presented = None;
        self.xui20_history_anchor = None;
        self.xui20_pending = None;
        self.xui20_pending_staged = false;
        self.xui20_pending_viewer_checkpoint = None;
        self.xui20_pending_cursor_checkpoint = None;
        self.xui20_cursor = None;
        self.xui21_presented = None;
        self.xui21_pending = None;
        self.xui21_pending_staged = false;
        self.xui21_pending_viewer_checkpoint = None;
        self.xui21_cursor = None;
        self.xui05_controller.reset();
        self.xui05_pending_owner = None;
        self.xui05_pending_controller_checkpoint = None;
        if let Some(authored) = self.authored_observer_ui.as_mut() {
            authored.invalidate_composed_frame();
        }
        self.execute_lifecycle.finish();
    }

    pub fn invalidate_for_resize_durably(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        let mut checkpoint = self.observer_frame_checkpoint.take();
        self.invalidate();
        viewer.politics.reset_frame_bound_state();
        viewer.peace_conference.invalidate_viewport_container();
        viewer.trade.active_scrollbar_drag = None;
        self.present_transaction
            .make_resize_invalidation_durable(viewer)?;
        let result = checkpoint
            .as_mut()
            .map(|checkpoint| checkpoint.make_runtime_invalidation_durable(viewer, self))
            .transpose()
            .map(|_| ());
        self.observer_frame_checkpoint = checkpoint;
        result
    }

    pub fn designer_effective_membership(
        &self,
        frame: &WorldFrame,
        selected_country: NamedCountry<'_>,
        viewer: &game_ui::design_catalog::ObserverDesignerViewState,
    ) -> Result<game_ui::designer_observer::ObserverDesignerEffectiveMembership> {
        self.renderer
            .xui13_effective_membership(frame, selected_country, viewer)
            .map_err(|error| format!("XUI13 effective membership refused: {error:?}").into())
    }

    pub fn open_authored_panel(
        &mut self,
        window_id: &str,
        bindings: crate::gui_definition_admission::AuthoredPanelViewerBindings,
        viewer_clock_micros: u64,
    ) -> Result<crate::gui_definition_admission::OpenedAuthoredPanel> {
        let authored = self
            .authored_observer_ui
            .as_mut()
            .ok_or("XUI02 authored observer UI is unavailable")?;
        authored.open_panel_with_bindings(
            &self.content_admission,
            window_id,
            bindings,
            viewer_clock_micros,
        )
    }

    pub fn close_authored_panel(&mut self, window_id: &str) -> Result<()> {
        let authored = self
            .authored_observer_ui
            .as_mut()
            .ok_or("XUI02 authored observer UI is unavailable")?;
        authored.close_panel(window_id)
    }

    pub fn cycle_frontend_panel(
        &mut self,
        world: &replay_core::WorldFrame,
        viewport_width: usize,
        viewport_height: usize,
        viewer_clock_micros: u64,
    ) -> Result<crate::gui_definition_admission::FrontendPanelRoute> {
        let authored = self
            .authored_observer_ui
            .as_mut()
            .ok_or("XUI26 authored observer UI is unavailable")?;
        authored.cycle_frontend_panel(
            &self.content_admission,
            world,
            viewport_width,
            viewport_height,
            viewer_clock_micros,
        )
    }

    pub fn close_xui26_frontend(&mut self) -> Result<bool> {
        let Some(authored) = self.authored_observer_ui.as_mut() else {
            return Ok(false);
        };
        authored.close_frontend_panels()
    }

    pub fn refresh_authored_panel(
        &mut self,
        window_id: &str,
        bindings: crate::gui_definition_admission::AuthoredPanelViewerBindings,
    ) -> Result<()> {
        let authored = self
            .authored_observer_ui
            .as_mut()
            .ok_or("XUI02 authored observer UI is unavailable")?;
        authored.refresh_open_panel_bindings(&self.content_admission, window_id, bindings)
    }

    pub fn authored_pointer_move(
        &mut self,
        x: i32,
        y: i32,
        viewer_clock_micros: u64,
    ) -> Result<Option<game_ui::window_manager_law::ReplayPointerTarget>> {
        let Some(authored) = self.authored_observer_ui.as_mut() else {
            return Ok(None);
        };
        authored.pointer_move(&self.content_admission, x, y, viewer_clock_micros)
    }

    pub fn xui15_pointer_press(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> Result<bool> {
        let Some(active) = self.present_transaction.presented_xui15() else {
            return Ok(false);
        };
        let panel = active.panel()?;
        if panel.validate_for_viewer(&viewer.trade).is_err() {
            return Ok(false);
        }
        let action = self
            .renderer
            .xui15_trade_pointer_action(
                panel,
                &mut viewer.trade,
                (x, y),
                active.viewport[0],
                active.viewport[1],
            )
            .map_err(|error| format!("XUI15 pointer admission refused: {error:?}"))?;
        if action == game_ui::trade_layout::TradePointerAction::None {
            return Ok(false);
        }
        if action == game_ui::trade_layout::TradePointerAction::Close {
            viewer.trade_panel_open = false;
            viewer.trade = game_ui::trade_layout::CountryTradeViewerState::default();
        }
        Ok(true)
    }

    pub fn xui15_wheel_notches_at(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
        emitted_notches: i32,
    ) -> Result<bool> {
        let Some(active) = self.present_transaction.presented_xui15() else {
            return Ok(false);
        };
        let panel = active.panel()?;
        if panel.validate_for_viewer(&viewer.trade).is_err() {
            return Ok(false);
        }
        let action = self
            .renderer
            .xui15_trade_wheel_action(
                panel,
                &mut viewer.trade,
                (x, y),
                emitted_notches,
                active.viewport[0],
                active.viewport[1],
            )
            .map_err(|error| format!("XUI15 wheel admission refused: {error:?}"))?;
        Ok(matches!(
            action,
            game_ui::trade_layout::TradePointerAction::ScrollCatalogue
                | game_ui::trade_layout::TradePointerAction::ScrollResourceFilters(_)
                | game_ui::trade_layout::TradePointerAction::ScrollCountryFilters(_)
        ))
    }

    pub fn xui15_has_pointer_capture(
        &self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
    ) -> bool {
        self.present_transaction.presented_xui15().is_some()
            && viewer.trade.active_scrollbar_drag.is_some()
    }

    pub fn xui15_pointer_move(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        pointer_y: i32,
    ) -> Result<bool> {
        let Some(active) = self.present_transaction.presented_xui15() else {
            viewer.trade.active_scrollbar_drag = None;
            return Ok(false);
        };
        if viewer.trade.active_scrollbar_drag.is_none() {
            return Ok(false);
        }
        let panel = active.panel()?;
        self.renderer
            .xui15_trade_drag_scrollbar(
                panel,
                &mut viewer.trade,
                pointer_y,
                active.viewport[0],
                active.viewport[1],
            )
            .map_err(|error| format!("XUI15 scrollbar drag refused: {error:?}").into())
    }

    pub fn xui15_pointer_release(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> bool {
        self.renderer
            .xui15_trade_release_scrollbar(&mut viewer.trade)
    }

    pub fn xui15_capture_lost(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> bool {
        let released = self
            .renderer
            .xui15_trade_release_scrollbar(&mut viewer.trade);
        if let Some(checkpoint) = self.present_transaction.viewer_checkpoint_mut() {
            self.renderer
                .xui15_trade_release_scrollbar(&mut checkpoint.trade);
        }
        released
    }

    pub fn xui15_blocks_lower_pointer_input(&self, x: i32, y: i32) -> Result<bool> {
        let Some(active) = self.present_transaction.presented_xui15() else {
            return Ok(false);
        };
        self.renderer
            .xui15_trade_blocks_lower_input((x, y), active.viewport[0], active.viewport[1])
            .map_err(|error| format!("XUI15 pointer-block admission refused: {error:?}").into())
    }

    pub fn production_lines_scroll_offset(&self) -> Result<Option<i32>> {
        let Some(authored) = self.authored_observer_ui.as_ref() else {
            return Ok(None);
        };
        authored.production_lines_scroll_offset()
    }

    pub fn authored_pointer_press(
        &mut self,
        x: i32,
        y: i32,
        viewer_clock_micros: u64,
    ) -> Result<Option<game_ui::window_manager_law::ReplayPointerTarget>> {
        let Some(authored) = self.authored_observer_ui.as_mut() else {
            return Ok(None);
        };
        authored.pointer_press(&self.content_admission, x, y, viewer_clock_micros)
    }

    pub fn authored_pointer_blocks_lower_input(&self, x: i32, y: i32) -> Result<bool> {
        let Some(authored) = self.authored_observer_ui.as_ref() else {
            return Ok(false);
        };
        authored.blocks_lower_pointer_input_at(x, y)
    }

    pub fn authored_pointer_release(
        &mut self,
        x: i32,
        y: i32,
        viewer_clock_micros: u64,
    ) -> Result<Option<game_ui::window_manager_law::ReplayPointerRelease>> {
        let Some(authored) = self.authored_observer_ui.as_mut() else {
            return Ok(None);
        };
        Ok(Some(authored.pointer_release(
            &self.content_admission,
            x,
            y,
            viewer_clock_micros,
        )?))
    }

    pub fn authored_keyboard_traverse(
        &mut self,
        direction: game_ui::window_manager_law::ReplayTraversalDirection,
    ) -> Result<bool> {
        let Some(authored) = self.authored_observer_ui.as_mut() else {
            return Ok(false);
        };
        authored.keyboard_traverse(direction)
    }

    pub fn authored_activate_focused(
        &mut self,
    ) -> Result<Option<game_ui::window_manager_law::AuthenticatedReplayViewerAction>> {
        let Some(authored) = self.authored_observer_ui.as_mut() else {
            return Ok(None);
        };
        authored.activate_focused_viewer_action()
    }

    pub fn authored_input_window_active(&self) -> bool {
        self.authored_observer_ui
            .as_ref()
            .is_some_and(crate::gui_definition_admission::AuthoredObserverUi::has_input_window)
    }

    pub fn cancel_authored_pointer_capture(&mut self) -> Result<()> {
        if let Some(authored) = self.authored_observer_ui.as_mut() {
            authored.cancel_pointer_capture()?;
        }
        Ok(())
    }

    pub fn cancel_authored_pointer_capture_durably(&mut self) -> Result<()> {
        self.cancel_authored_pointer_capture()?;
        if let Some(checkpoint) = self.observer_frame_checkpoint.as_mut() {
            if let Some(authored) = checkpoint.authored_input.as_mut() {
                authored.apply_capture_loss();
            }
        }
        Ok(())
    }

    pub fn authenticated_authored_tooltip_owner_at_cursor(
        &self,
    ) -> Result<Option<game_ui::window_manager_law::AuthenticatedReplayTooltipOwner>> {
        let Some(authored) = self.authored_observer_ui.as_ref() else {
            return Ok(None);
        };
        authored.authenticated_tooltip_owner_at_cursor()
    }

    pub fn set_authenticated_authored_tooltip_owner(
        &mut self,
        owner: &game_ui::window_manager_law::AuthenticatedReplayTooltipOwner,
        tooltip_instance_record: &str,
    ) -> Result<()> {
        let authored = self
            .authored_observer_ui
            .as_mut()
            .ok_or("XUI02 authored observer UI is unavailable")?;
        authored.set_authenticated_tooltip_owner(owner, tooltip_instance_record)
    }

    pub fn scroll_authored_panel_at(
        &mut self,
        x: i32,
        y: i32,
        delta_pixels: game_ui::window_manager_law::ReplayScrollDeltaPixels,
    ) -> Result<game_ui::window_manager_law::ReplayScrollOutcome> {
        let authored = self
            .authored_observer_ui
            .as_mut()
            .ok_or("XUI02 authored observer UI is unavailable")?;
        authored.scroll_at(x, y, delta_pixels)
    }

    pub fn scroll_authored_wheel_notches_at(
        &mut self,
        x: i32,
        y: i32,
        emitted_notches: i32,
    ) -> Result<Option<game_ui::window_manager_law::ReplayScrollOutcome>> {
        let authored = self
            .authored_observer_ui
            .as_mut()
            .ok_or("XUI02 authored observer UI is unavailable")?;
        authored.scroll_wheel_notches_at(x, y, emitted_notches)
    }

    pub fn production_lines_scroll_offset_for_outcome(
        &self,
        outcome: &game_ui::window_manager_law::ReplayScrollOutcome,
    ) -> Result<Option<i32>> {
        let authored = self
            .authored_observer_ui
            .as_ref()
            .ok_or("XUI02 authored observer UI is unavailable")?;
        authored.production_lines_scroll_offset_for_outcome(outcome)
    }

    pub fn authored_window_snapshot(
        &mut self,
        viewer_clock_micros: u64,
    ) -> Result<Option<game_ui::window_manager_law::ReplayWindowManagerSnapshot>> {
        let Some(authored) = self.authored_observer_ui.as_mut() else {
            return Ok(None);
        };
        Ok(Some(authored.current_window_snapshot(
            &self.content_admission,
            viewer_clock_micros,
        )?))
    }

    pub fn confirm_presented_authored_frame(&mut self) -> Result<()> {
        let authored = self
            .authored_observer_ui
            .as_mut()
            .ok_or("XUI02 authored observer UI is unavailable")?;
        authored.confirm_presented_frame()?;
        self.execute_lifecycle.finish();
        if let Some((owner, record)) = self.xui05_pending_owner.take() {
            let current = authored.authenticated_tooltip_owner_at_cursor()?;
            let matches = current.as_ref() == Some(&owner);
            if matches {
                authored.set_authenticated_tooltip_owner(
                    current.as_ref().expect("matched current tooltip owner"),
                    &record,
                )?;
            } else {
                authored.clear_authenticated_tooltip_owner()?;
            }
        } else {
            authored.clear_authenticated_tooltip_owner()?;
        }
        self.xui05_pending_controller_checkpoint = None;
        self.xui23_presented = self.xui23_pending.take();
        self.xui10_presented = self.xui10_pending.take();
        if self.xui09_pending_staged {
            self.xui09_presented = self.xui09_pending.take();
            self.xui09_pending_staged = false;
            self.xui09_pending_viewer_checkpoint = None;
        }
        if self.xui11_pending_staged {
            let pending = self.xui11_pending.take();
            if let Some(presented) = pending.as_ref() {
                self.xui11_history_anchor = Some(Xui11HistoryAnchor::from(presented));
            }
            self.xui11_presented = pending;
            self.xui11_pending_staged = false;
            self.xui11_pending_viewer_checkpoint = None;
        }
        if self.xui18_pending_staged {
            self.xui18_presented = self.xui18_pending.take();
            self.xui18_pending_staged = false;
            self.xui18_pending_viewer_checkpoint = None;
        }
        if self.xui19_pending_staged {
            let pending = self.xui19_pending.take();
            if let Some(presented) = pending.as_ref() {
                self.xui19_history_anchor = Some(Xui19HistoryAnchor::from(presented));
            }
            self.xui19_presented = pending;
            self.xui19_pending_staged = false;
            self.xui19_pending_viewer_checkpoint = None;
        }
        if self.xui08_pending_staged {
            let pending = commit_xui08_staged_candidate(
                &mut self.xui08_pending,
                &mut self.xui08_pending_staged,
                &mut self.xui08_pending_viewer_checkpoint,
            )?;
            if let Some(presented) = pending.as_ref() {
                self.xui08_history_anchor = Some(Xui08HistoryAnchor::from(presented));
            }
            self.xui08_presented = pending;
        }
        self.xui08_pending_viewer_input = None;
        if self.xui14_pending_staged {
            let pending = self.xui14_pending.take();
            if let Some(presented) = pending.as_ref() {
                self.xui14_history_anchor = Some(Xui14HistoryAnchor::from(presented));
            }
            self.xui14_presented = pending;
            self.xui14_pending_staged = false;
            self.xui14_pending_viewer_checkpoint = None;
        }
        if self.xui20_pending_staged {
            let pending = self.xui20_pending.take();
            if let Some(presented) = pending.as_ref() {
                self.xui20_history_anchor = Some(Xui20HistoryAnchor::from(presented));
            }
            self.xui20_presented = pending;
            self.xui20_pending_staged = false;
            self.xui20_pending_viewer_checkpoint = None;
            self.xui20_pending_cursor_checkpoint = None;
        }
        if self.xui21_pending_staged {
            self.xui21_presented = commit_xui21_staged_candidate(
                &mut self.xui21_pending_viewer_checkpoint,
                &mut self.xui21_pending,
                &mut self.xui21_pending_staged,
            )?;
        }
        self.observer_frame_checkpoint
            .take()
            .ok_or("observer UI frame checkpoint is absent for presented frame")?
            .commit();
        Ok(())
    }

    pub fn confirm_presented_without_observer_ui(&mut self) -> Result<()> {
        self.observer_frame_checkpoint
            .take()
            .ok_or("observer UI frame checkpoint is absent for unrendered Presented frame")?
            .commit();
        Ok(())
    }

    pub(crate) fn finish_observer_frame_disposition(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        disposition: ObserverUiFrameDisposition,
        observer_ui_executed: bool,
        production_lines_scroll_offset: Option<i32>,
    ) -> Result<()> {
        match disposition {
            ObserverUiFrameDisposition::Presented => {
                if observer_ui_executed {
                    self.confirm_presented_authored_frame()?;
                } else {
                    self.confirm_presented_without_observer_ui()?;
                }
                self.present_transaction.finish_disposition(
                    viewer,
                    disposition,
                    production_lines_scroll_offset,
                )?;
                if !observer_ui_executed {
                    self.invalidate();
                }
            }
            ObserverUiFrameDisposition::Skipped
            | ObserverUiFrameDisposition::Occluded
            | ObserverUiFrameDisposition::ExecuteError => {
                if observer_ui_executed {
                    self.reject_unpresented_authored_frame(viewer)?;
                } else {
                    self.reject_unexecuted_observer_frame(viewer)?;
                }
            }
        }
        Ok(())
    }

    pub fn reject_unexecuted_observer_frame(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        self.observer_frame_checkpoint
            .take()
            .ok_or("observer UI frame checkpoint is absent for unexecuted rejection")?
            .restore(viewer, self)?;
        self.xui08_pending_viewer_input = None;
        self.present_transaction.reject(viewer)?;
        Ok(())
    }

    fn clear_execute_candidates(&mut self) {
        self.present_transaction.xui15_pending = None;
        self.xui05_pending_owner = None;
        self.xui05_pending_controller_checkpoint = None;
        self.xui23_pending = None;
        self.xui10_pending = None;
        self.xui09_pending = None;
        self.xui09_pending_staged = false;
        self.xui09_pending_viewer_checkpoint = None;
        self.xui11_pending = None;
        self.xui11_pending_staged = false;
        self.xui11_pending_viewer_checkpoint = None;
        self.xui18_pending = None;
        self.xui18_pending_staged = false;
        self.xui18_pending_viewer_checkpoint = None;
        self.xui19_pending = None;
        self.xui19_pending_staged = false;
        self.xui19_pending_viewer_checkpoint = None;
        self.xui08_pending = None;
        self.xui08_pending_staged = false;
        self.xui08_pending_viewer_checkpoint = None;
        self.xui08_pending_viewer_input = None;
        self.xui14_pending = None;
        self.xui14_pending_staged = false;
        self.xui14_pending_viewer_checkpoint = None;
        self.xui20_pending = None;
        self.xui20_pending_staged = false;
        self.xui20_pending_viewer_checkpoint = None;
        self.xui20_pending_cursor_checkpoint = None;
        self.xui21_pending = None;
        self.xui21_pending_staged = false;
        self.xui21_pending_viewer_checkpoint = None;
    }

    pub fn rollback_after_execute_error(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        let frame_pending = self.observer_frame_checkpoint.is_some();
        let owner_pending = self.present_transaction.has_pending_disposition();
        if self
            .execute_lifecycle
            .finish_ownerless_execute_error(frame_pending, owner_pending)?
        {
            self.clear_execute_candidates();
            return Ok(());
        }
        let frame_checkpoint = self
            .observer_frame_checkpoint
            .take()
            .ok_or("observer UI execute rollback lost its frame checkpoint")?;
        let authored_result = if self.execute_lifecycle.authored_frame_staged() {
            match self.authored_observer_ui.as_mut() {
                Some(authored) => authored.reject_unpresented_frame(),
                None => Err("XUI02 staged frame lost its authored runtime".into()),
            }
        } else {
            Ok(())
        };
        self.execute_lifecycle.finish();
        self.clear_execute_candidates();
        let frame_restore_result = frame_checkpoint.restore(viewer, self);
        let viewer_restore_result = self.present_transaction.reject(viewer);
        authored_result?;
        frame_restore_result?;
        viewer_restore_result
    }

    pub fn reject_unpresented_authored_frame(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        let frame_checkpoint = self
            .observer_frame_checkpoint
            .take()
            .ok_or("observer UI frame checkpoint is absent for rejected frame")?;
        let reject_result = (|| -> Result<()> {
            self.present_transaction.xui15_pending = None;
            self.xui05_pending_owner = None;
            self.xui23_pending = None;
            self.xui10_pending = None;
            self.xui09_pending = None;
            self.xui09_pending_staged = false;
            self.xui11_pending = None;
            self.xui11_pending_staged = false;
            self.xui18_pending = None;
            self.xui18_pending_staged = false;
            self.xui19_pending = None;
            self.xui19_pending_staged = false;
            self.xui14_pending = None;
            self.xui14_pending_staged = false;
            viewer.research = self
                .xui09_pending_viewer_checkpoint
                .take()
                .ok_or("XUI09 viewer checkpoint is absent for rejected frame")?;
            self.xui09_drag_cursor = None;
            viewer.politics = self
                .xui11_pending_viewer_checkpoint
                .take()
                .ok_or("XUI11 viewer checkpoint is absent for rejected frame")?;
            viewer.power_balance = self
                .xui18_pending_viewer_checkpoint
                .take()
                .ok_or("XUI18 viewer checkpoint is absent for rejected frame")?;
            let (decision_open, decision_scroll, decision_collapsed) = self
                .xui19_pending_viewer_checkpoint
                .take()
                .ok_or("XUI19 viewer checkpoint is absent for rejected frame")?;
            viewer.decision_panel_open = decision_open;
            viewer.decision_scroll_px = decision_scroll;
            viewer.collapsed_decision_categories = decision_collapsed;
            rollback_xui08_staged_candidate(
                &mut viewer.peace_conference,
                &mut self.xui08_pending_viewer_checkpoint,
                &mut self.xui08_pending,
                &mut self.xui08_pending_staged,
            )?;
            viewer.intelligence = self
                .xui14_pending_viewer_checkpoint
                .take()
                .ok_or("XUI14 viewer checkpoint is absent for rejected frame")?;
            rollback_xui20_staged_candidate(
                &mut viewer.combat,
                &mut self.xui20_pending_viewer_checkpoint,
                &mut self.xui20_cursor,
                &mut self.xui20_pending_cursor_checkpoint,
                &mut self.xui20_pending,
                &mut self.xui20_pending_staged,
            )?;
            if self.xui21_pending_viewer_checkpoint.is_none() {
                return Err("XUI21 viewer checkpoint is absent for rejected frame".into());
            }
            rollback_xui21_staged_candidate(
                &mut viewer.events,
                &mut self.xui21_pending_viewer_checkpoint,
                &mut self.xui21_pending,
                &mut self.xui21_pending_staged,
            )?;
        if self.execute_lifecycle.authored_frame_staged() {
                let authored = self
                    .authored_observer_ui
                    .as_mut()
                    .ok_or("XUI02 authored observer UI is unavailable")?;
                authored.reject_unpresented_frame()?;
            self.execute_lifecycle.finish();
        }
            self.xui05_controller = self
                .xui05_pending_controller_checkpoint
                .take()
                .ok_or("XUI05 controller checkpoint is absent for rejected frame")?;
            Ok(())
        })();
        frame_checkpoint.restore(viewer, self)?;
        self.present_transaction.reject(viewer)?;
        reject_result
    }

    pub fn toggle_xui19(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        if viewer.decision_panel_open {
            viewer.decision_panel_open = false;
            viewer.decision_scroll_px = 0;
            viewer.collapsed_decision_categories.clear();
            return Ok(());
        }
        xui19_matching_presented(viewer, self.xui19_presented.as_ref())
            .ok_or("XUI19 has no matching Presented passive all-country carrier")?;
        viewer.decision_panel_open = true;
        viewer.decision_scroll_px = 0;
        viewer.collapsed_decision_categories.clear();
        Ok(())
    }

    pub fn close_xui19(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) {
        viewer.decision_panel_open = false;
        viewer.decision_scroll_px = 0;
        viewer.collapsed_decision_categories.clear();
        self.xui19_cursor = None;
    }

    pub fn xui19_pointer_move(&mut self, x: i32, y: i32) {
        self.xui19_cursor = Some((x, y));
    }

    pub fn xui19_blocks_lower_pointer_input(
        &self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> bool {
        if !viewer.decision_panel_open {
            return false;
        }
        match xui19_matching_presented(viewer, self.xui19_presented.as_ref())
            .filter(|frame| frame.panel_drawn)
            .and_then(|frame| frame.panel_rect)
        {
            Some(rect) => xui19_rect_contains(rect, x, y),
            None => true,
        }
    }

    pub fn xui19_pointer_press(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> Result<ObserverViewerActionDisposition> {
        let presented = xui19_matching_presented(viewer, self.xui19_presented.as_ref())
            .filter(|frame| frame.panel_drawn)
            .ok_or("XUI19 has no matching Presented composed panel")?;
        if let Some(category) = presented
            .category_extents
            .iter()
            .find(|category| xui19_rect_contains(category.clipped_header_rect, x, y))
        {
            if category.expanded {
                viewer
                    .collapsed_decision_categories
                    .insert(category.category_id.clone());
            } else {
                viewer
                    .collapsed_decision_categories
                    .remove(&category.category_id);
            }
            viewer.decision_scroll_px = 0;
            return Ok(ObserverViewerActionDisposition::Applied);
        }
        if let Some(widget) = presented
            .xui22_widget_hit_targets
            .iter()
            .rev()
            .find(|widget| {
                (widget.visual_enabled.is_some()
                    || widget.click_enabled.is_some()
                    || widget.right_click_enabled.is_some()
                    || !widget.effect_names.is_empty())
                    && xui19_rect_contains(widget.rect, x, y)
            })
        {
            return Ok(
                ObserverViewerActionDisposition::RefusedDecisionCategoryProviderActivation {
                    category_id: widget.category_id.clone(),
                    provider_id: widget.provider_id.clone(),
                    stable_namespace: widget.stable_namespace.clone(),
                },
            );
        }
        if let Some(card) = presented.provider_receipts.iter().rev().find(|receipt| {
            xui19_provider_screen_rect(presented, receipt)
                .is_some_and(|rect| xui19_rect_contains(rect, x, y))
        }) {
            return Ok(
                ObserverViewerActionDisposition::RefusedDecisionCategoryProviderActivation {
                    category_id: card.category_id.clone(),
                    provider_id: card.provider_id.clone(),
                    stable_namespace: card.stable_namespace.clone(),
                },
            );
        }
        if let Some(card) = presented
            .card_hit_targets
            .iter()
            .rev()
            .find(|card| xui19_rect_contains(card.rect, x, y))
        {
            return Ok(ObserverViewerActionDisposition::RefusedDecisionActivation {
                source_record_id: card.source_record_id.clone(),
                decision_id: card.decision_id.clone(),
            });
        }
        Ok(ObserverViewerActionDisposition::Applied)
    }

    pub fn xui19_page_scroll(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        forward: bool,
    ) -> Result<()> {
        let presented = xui19_matching_presented(viewer, self.xui19_presented.as_ref())
            .filter(|frame| frame.panel_drawn)
            .ok_or("XUI19 has no matching Presented composed panel")?;
        let page = presented
            .decision_grid_rect
            .map(|rect| rect[3])
            .filter(|height| *height > 0)
            .ok_or("XUI19 Presented decision grid has no positive page extent")?;
        viewer.decision_scroll_px = if forward {
            viewer
                .decision_scroll_px
                .saturating_add(page)
                .min(presented.max_scroll_px)
        } else {
            viewer.decision_scroll_px.saturating_sub(page)
        };
        Ok(())
    }

    pub fn xui19_wheel_notches_at(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
        emitted_notches: i32,
    ) -> Result<bool> {
        if emitted_notches == 0 || !viewer.decision_panel_open {
            return Ok(false);
        }
        let presented = xui19_matching_presented(viewer, self.xui19_presented.as_ref())
            .filter(|frame| frame.panel_drawn)
            .ok_or("XUI19 open panel has no matching Presented composed geometry")?;
        let grid = presented
            .decision_grid_rect
            .ok_or("XUI19 Presented decision grid is absent")?;
        if !xui19_rect_contains(grid, x, y) {
            return Ok(false);
        }
        let page = grid[3]
            .checked_mul(i32::try_from(emitted_notches.unsigned_abs()).unwrap_or(i32::MAX))
            .unwrap_or(i32::MAX);
        viewer.decision_scroll_px = if emitted_notches < 0 {
            viewer
                .decision_scroll_px
                .saturating_add(page)
                .min(presented.max_scroll_px)
        } else {
            viewer.decision_scroll_px.saturating_sub(page)
        };
        Ok(true)
    }

    pub fn xui21_pointer_move(&mut self, x: i32, y: i32) {
        self.xui21_cursor = Some((x, y));
    }

    pub fn xui21_blocks_lower_pointer_input(
        &self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> bool {
        xui21_matching_presented_frame(
            &viewer.selected_country,
            self.xui21_presented.as_ref(),
        )
        .is_some_and(|frame| {
            frame
                .popups
                .iter()
                .rev()
                .any(|popup| popup.root_region.contains(x, y))
        })
    }

    pub fn xui21_pointer_press(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> Result<ObserverViewerActionDisposition> {
        let presented = xui21_matching_presented_frame(
            &viewer.selected_country,
            self.xui21_presented.as_ref(),
        )
        .ok_or("XUI21 has no matching Presented event frame")?;
        press_xui21_presented_popup(presented, &mut viewer.events, x, y)
    }

    pub fn cycle_xui21_focus(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        backwards: bool,
    ) -> Result<()> {
        let presented = xui21_matching_presented_frame(
            &viewer.selected_country,
            self.xui21_presented.as_ref(),
        )
        .ok_or("XUI21 has no matching Presented event frame")?;
        cycle_xui21_presented_focus(presented, &mut viewer.events, backwards)
    }

    pub fn dismiss_xui21_focused(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        let presented = xui21_matching_presented_frame(
            &viewer.selected_country,
            self.xui21_presented.as_ref(),
        )
        .ok_or("XUI21 has no matching Presented event frame")?;
        dismiss_xui21_presented_focus(presented, &mut viewer.events)
    }

    pub fn open_xui23_context(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        context: game_ui::active_abilities::ObserverAbilityContext,
    ) -> Result<()> {
        let presented = self
            .xui23_presented
            .as_ref()
            .ok_or("XUI23 has no Presented authenticated frame")?;
        if presented.country_tag != viewer.selected_country
            || !presented.contexts.contains(&context)
        {
            return Err(
                "XUI23 context is stale or absent from the Presented country roster".into(),
            );
        }
        viewer.abilities.open = true;
        viewer.abilities.selected_context = Some(context);
        Ok(())
    }

    pub fn close_xui23(&self, viewer: &mut game_ui::production_renderer::ObserverViewerUiState) {
        viewer.abilities.open = false;
        viewer.abilities.selected_context = None;
    }

    pub fn cycle_xui23_context(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        let presented = self
            .xui23_presented
            .as_ref()
            .ok_or("XUI23 has no Presented authenticated frame")?;
        if presented.country_tag != viewer.selected_country || presented.contexts.is_empty() {
            return Err("XUI23 Presented context roster is unavailable for this country".into());
        }
        if !viewer.abilities.open || viewer.abilities.selected_context.is_none() {
            return self.open_xui23_context(viewer, presented.contexts[0].clone());
        }
        let Some(selected) = viewer.abilities.selected_context.as_ref() else {
            return Err("XUI23 open state has no selected Presented context".into());
        };
        let index = presented
            .contexts
            .iter()
            .position(|context| context == selected)
            .ok_or("XUI23 selected context is stale for the Presented frame")?;
        if let Some(next) = presented.contexts.get(index.saturating_add(1)) {
            self.open_xui23_context(viewer, next.clone())
        } else {
            self.close_xui23(viewer);
            Ok(())
        }
    }

    pub fn reset_country_dependent_ui_for_country_change(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) {
        if let Some(authored) = self.authored_observer_ui.as_mut() {
            if let Err(error) = authored.close_frontend_panels() {
                eprintln!("replay_app: XUI26 country reset failed closed: {error}");
            }
        }
        viewer.production_lines.reset_for_seek_or_country_change();
        viewer.abilities.reset_for_seek();
        viewer.focus.reset_for_seek_or_country_change();
        viewer.politics.reset_frame_bound_state();
        viewer.power_balance.reset_for_seek_or_country_change();
        self.xui23_presented = None;
        self.xui10_presented = None;
        self.xui10_drag_cursor = None;
        self.reset_xui09_for_country_change(viewer);
        self.xui11_presented = None;
        self.xui11_history_anchor = None;
        self.xui11_pending = None;
        self.xui11_pending_staged = false;
        self.xui11_pending_viewer_checkpoint = None;
        self.xui11_cursor = None;
        self.xui18_presented = None;
        self.xui18_pending = None;
        self.xui18_pending_staged = false;
        self.xui18_pending_viewer_checkpoint = None;
        self.xui18_cursor = None;
        viewer.decision_panel_open = false;
        viewer.decision_scroll_px = 0;
        viewer.collapsed_decision_categories.clear();
        self.xui19_presented = None;
        self.xui19_history_anchor = None;
        self.xui19_pending = None;
        self.xui19_pending_staged = false;
        self.xui19_pending_viewer_checkpoint = None;
        self.xui19_cursor = None;
        self.xui14_presented = None;
        self.xui14_history_anchor = None;
        self.xui14_pending = None;
        self.xui14_pending_staged = false;
        self.xui14_pending_viewer_checkpoint = None;
        self.xui14_cursor = None;
        self.xui17_cursor = None;
        viewer.combat.reset_for_seek();
        self.xui20_presented = None;
        self.xui20_history_anchor = None;
        self.xui20_pending = None;
        self.xui20_pending_staged = false;
        self.xui20_pending_viewer_checkpoint = None;
        self.xui20_pending_cursor_checkpoint = None;
        self.xui20_cursor = None;
        viewer.events.reset_for_seek_or_country_change();
        self.xui21_presented = None;
        self.xui21_pending = None;
        self.xui21_pending_staged = false;
        self.xui21_pending_viewer_checkpoint = None;
        self.xui21_cursor = None;
    }

    pub fn reset_xui23_for_seek(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) {
        if let Some(authored) = self.authored_observer_ui.as_mut() {
            if let Err(error) = authored.close_frontend_panels() {
                eprintln!("replay_app: XUI26 seek reset failed closed: {error}");
            }
        }
        self.reset_country_dependent_ui_for_country_change(viewer);
        viewer.peace_conference = Default::default();
        self.xui08_presented = None;
        self.xui08_history_anchor = None;
        self.xui08_pending = None;
        self.xui08_pending_staged = false;
        self.xui08_pending_viewer_checkpoint = None;
        self.xui08_pending_viewer_input = None;
        self.xui08_cursor = None;
    }

    pub fn open_xui20_panel(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        panel: game_ui::combat_detail_canvas::Xui20ViewerPanel,
    ) -> Result<()> {
        let presented = self
            .xui20_presented
            .as_ref()
            .ok_or("XUI20 has no Presented passive-global carrier")?;
        if presented.record_sha256.is_empty() {
            return Err("XUI20 Presented passive carrier identity is empty".into());
        }
        if !presented
            .available_panels
            .iter()
            .any(|available| available.panel == panel)
        {
            return Err("XUI20 selected panel identity is absent from the Presented frame".into());
        }
        if let Some(country_tag) = panel.country_tag() {
            if !viewer.selected_country.is(country_tag) {
                return Err(
                    "XUI20 country panel differs from the observer-selected country".into(),
                );
            }
        }
        let frame_hour = i64::try_from(presented.frame_hour)
            .map_err(|_| "XUI20 Presented frame hour exceeds replay viewer range")?;
        viewer.combat.open(panel, frame_hour);
        Ok(())
    }

    pub fn close_xui20(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) {
        viewer.combat.close();
        self.xui20_cursor = None;
    }

    pub fn cycle_xui20_combat_log_tab(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        use game_ui::combat_detail_canvas::{Xui20CombatLogTab, Xui20ViewerPanel};
        let Xui20ViewerPanel::CombatLog { country_tag, tab } = viewer
            .combat
            .panel
            .clone()
            .ok_or("XUI20 combat-log tab cycle requires an open panel")?
        else {
            return Err("XUI20 tab cycle requires the combat-log panel".into());
        };
        let tab = match tab {
            Xui20CombatLogTab::Combats => Xui20CombatLogTab::Equipment,
            Xui20CombatLogTab::Equipment => Xui20CombatLogTab::Templates,
            Xui20CombatLogTab::Templates => Xui20CombatLogTab::Combats,
        };
        self.open_xui20_panel(viewer, Xui20ViewerPanel::CombatLog { country_tag, tab })
    }

    pub fn cycle_xui20_panel(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        let presented = self
            .xui20_presented
            .as_ref()
            .ok_or("XUI20 has no Presented passive-global carrier")?;
        let eligible = presented
            .available_panels
            .iter()
            .map(|available| &available.panel)
            .filter(|panel| {
                panel.country_tag().is_none()
                    || panel
                        .country_tag()
                        .is_some_and(|tag| viewer.selected_country.is(tag))
            })
            .cloned()
            .collect::<Vec<_>>();
        if eligible.is_empty() {
            return Err("XUI20 Presented frame has no panel eligible for this observer".into());
        }
        let next = viewer
            .combat
            .panel
            .as_ref()
            .and_then(|current| eligible.iter().position(|panel| panel == current))
            .map_or(0, |index| (index + 1) % eligible.len());
        self.open_xui20_panel(viewer, eligible[next].clone())
    }

    pub fn xui20_blocks_lower_pointer_input(&self, x: i32, y: i32) -> bool {
        self.xui20_presented
            .as_ref()
            .and_then(|presented| presented.input_region)
            .is_some_and(|region| region.panel.contains_half_open((x, y)))
    }

    pub fn xui20_pointer_press(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> bool {
        let Some(region) = self
            .xui20_presented
            .as_ref()
            .and_then(|presented| presented.input_region)
        else {
            return false;
        };
        if viewer.combat.panel.is_none() || !region.panel.contains_half_open((x, y)) {
            return false;
        }
        if region.close.contains_half_open((x, y)) {
            self.close_xui20(viewer);
        }
        true
    }

    pub fn xui20_wheel_notches_at(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
        emitted_notches: i32,
    ) -> Result<bool> {
        let Some(region) = self
            .xui20_presented
            .as_ref()
            .and_then(|presented| presented.input_region)
        else {
            return Ok(false);
        };
        if viewer.combat.panel.is_none() || !region.panel.contains_half_open((x, y)) {
            return Ok(false);
        }
        let Some(channel) = region.scroll_channel_at((x, y)) else {
            return Ok(true);
        };
        let (wheel_px, maximum_px) = match channel {
            replay_core::CombatScrollChannelExact::Primary => {
                (region.primary_scroll_wheel_px, region.primary_scroll_max_px)
            }
            replay_core::CombatScrollChannelExact::Secondary => (
                region.secondary_scroll_wheel_px,
                region.secondary_scroll_max_px,
            ),
            replay_core::CombatScrollChannelExact::None => {
                return Err("XUI20 input region returned a non-scroll channel".into())
            }
        };
        viewer
            .combat
            .wheel(channel, emitted_notches, wheel_px, maximum_px)?;
        Ok(true)
    }

    pub fn xui20_pointer_move(&mut self, x: i32, y: i32) {
        self.xui20_cursor = Some((x, y));
    }

    pub fn xui17_pointer_move(&mut self, x: i32, y: i32) {
        self.xui17_cursor = Some((x, y));
    }

    pub fn toggle_xui14(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        if viewer.intelligence.panel_open {
            viewer.intelligence.close();
            return Ok(());
        }
        // A COMPARISON: there is no frame in an input handler, and the
        // presented carrier below carries a name its own frame admitted.
        if !viewer.selected_country.is_selected() {
            return Err("XUI14 cannot open without an observer-selected country".into());
        }
        let country = &viewer.selected_country;
        let presented = self
            .xui14_presented
            .as_ref()
            .filter(|presented| presented.country_tags.iter().any(|tag| country.is(tag)))
            .ok_or("XUI14 has no Presented passive carrier for the selected country")?;
        if presented.record_sha256.is_empty() {
            return Err("XUI14 Presented passive carrier identity is empty".into());
        }
        let admitted = presented
            .country_tags
            .iter()
            .find(|tag| country.is(tag))
            .expect("the filter above matched this carrier on the selection")
            .clone();
        viewer.intelligence.toggle(Some(&admitted));
        Ok(())
    }

    pub fn cycle_xui14_surface(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        if !viewer.intelligence.panel_open {
            return Err("XUI14 surface cycle requires an open Presented panel".into());
        }
        viewer
            .intelligence
            .select_surface(viewer.intelligence.surface.next());
        Ok(())
    }

    pub fn xui14_blocks_lower_pointer_input(
        &self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> bool {
        viewer.intelligence.panel_open
            && self
                .xui14_presented
                .as_ref()
                .and_then(|presented| presented.input_region)
                .is_some_and(|region| region.panel_contains(x, y))
    }

    pub fn xui14_pointer_press(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> Result<bool> {
        let Some(presented) = self.xui14_presented.as_ref() else {
            return Ok(false);
        };
        let Some(region) = presented.input_region else {
            return Ok(false);
        };
        if !viewer.intelligence.panel_open || !region.panel_contains(x, y) {
            return Ok(false);
        }
        if region.close_contains(x, y) {
            viewer.intelligence.close();
        } else if region.operations_contains(x, y) {
            viewer.intelligence.select_surface(
                game_ui::intelligence_presentation::IntelligenceViewerSurface::Operations,
            );
        } else if region.crypto_contains(x, y) {
            viewer.intelligence.select_surface(
                game_ui::intelligence_presentation::IntelligenceViewerSurface::Cryptology,
            );
        }
        Ok(true)
    }

    pub fn xui14_wheel_notches_at(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
        emitted_notches: i32,
    ) -> bool {
        let Some(presented) = self.xui14_presented.as_ref() else {
            return false;
        };
        let Some(region) = presented.input_region else {
            return false;
        };
        if !viewer.intelligence.panel_open || region.row_count <= 1 || !region.panel_contains(x, y)
        {
            return false;
        }
        viewer.intelligence.wheel(emitted_notches, region.row_count);
        true
    }

    pub fn xui14_pointer_move(&mut self, x: i32, y: i32) {
        self.xui14_cursor = Some((x, y));
    }

    fn stage_xui08_viewer_input<'a>(
        &'a mut self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
    ) -> &'a mut game_ui::peace_conference_ui::PeaceConferenceViewerState {
        stage_xui08_viewer_input(
            &mut self.xui08_pending_viewer_input,
            &viewer.peace_conference,
        )
    }

    /// passive carrier. Recorder window ownership is never consulted. Input
    pub fn toggle_xui08(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        if self
            .xui08_pending_viewer_input
            .as_ref()
            .unwrap_or(&viewer.peace_conference)
            .open
        {
            self.stage_xui08_viewer_input(viewer).set_open(false);
            return Ok(());
        }
        let presented = self
            .xui08_presented
            .as_ref()
            .filter(|presented| presented.admitted.active())
            .ok_or("XUI08 has no Presented active passive-global conference")?;
        if presented.record_sha256.is_empty() {
            return Err("XUI08 Presented passive carrier identity is empty".into());
        }
        let admitted = presented.admitted.clone();
        let staged = self.stage_xui08_viewer_input(viewer);
        staged.set_open(true);
        staged.synchronize(&admitted);
        Ok(())
    }

    pub fn close_xui08(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) {
        stage_xui08_closed(
            &mut self.xui08_pending_viewer_input,
            &viewer.peace_conference,
        );
        self.xui08_cursor = None;
    }

    pub fn close_xui08_topmost(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> bool {
        let current = self
            .xui08_pending_viewer_input
            .as_ref()
            .unwrap_or(&viewer.peace_conference);
        if !current.open {
            return false;
        }
        let staged = self.stage_xui08_viewer_input(viewer);
        let closed = close_xui08_topmost_state(staged);
        if closed && staged.open {
            // Popup-only close retains panel cursor ownership.
        } else {
            self.xui08_cursor = None;
        }
        closed
    }

    pub fn xui08_will_be_open(
        &self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
    ) -> bool {
        xui08_current_viewer_state(&self.xui08_pending_viewer_input, &viewer.peace_conference).open
    }

    pub fn xui08_blocks_lower_pointer_input(
        &self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> bool {
        let current =
            xui08_current_viewer_state(&self.xui08_pending_viewer_input, &viewer.peace_conference);
        xui08_blocks_lower_pointer_input_state(
            current,
            self.xui08_presented
                .as_ref()
                .and_then(|presented| presented.input_region.as_ref()),
            x,
            y,
        )
    }

    pub fn xui08_pointer_move(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> Result<()> {
        if self.xui08_will_be_open(viewer) {
            self.xui08_cursor = Some((x, y));
            if let Some(region) = self
                .xui08_presented
                .as_ref()
                .and_then(|presented| presented.input_region.as_ref())
                .cloned()
            {
                self.stage_xui08_viewer_input(viewer)
                    .drag_active_scrollbar(&region, y)?;
            }
        }
        Ok(())
    }

    pub fn xui08_pointer_release(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> bool {
        release_xui08_pointer_capture(
            &mut self.xui08_pending_viewer_input,
            &viewer.peace_conference,
        )
    }

    pub fn xui08_capture_lost(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> bool {
        let released = release_xui08_pointer_capture_durably(
            &mut self.xui08_pending_viewer_input,
            &mut viewer.peace_conference,
        );
        if let (Some(checkpoint), Some(viewer_checkpoint)) = (
            self.observer_frame_checkpoint.as_mut(),
            self.present_transaction.viewer_checkpoint_mut(),
        ) {
            release_xui08_pointer_capture_durably(
                &mut checkpoint.xui08_pending_viewer_input,
                &mut viewer_checkpoint.peace_conference,
            );
        }
        released
    }

    pub fn xui10_capture_lost(&mut self) -> bool {
        release_xui10_pointer_capture_durably(
            &mut self.xui10_drag_cursor,
            &mut self.observer_frame_checkpoint,
        )
    }

    pub fn xui08_wheel_notches_at(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
        emitted_notches: i32,
    ) -> Result<bool> {
        let Some(presented) = self.xui08_presented.as_ref() else {
            return Ok(false);
        };
        let Some(region) = presented.input_region.clone() else {
            return Ok(false);
        };
        let current =
            xui08_current_viewer_state(&self.xui08_pending_viewer_input, &viewer.peace_conference);
        if !current.open || !region.panel_contains((x, y)) {
            return Ok(false);
        }
        let Some(scroll_region) = region.region_at((x, y)) else {
            return Ok(true);
        };
        let Some(step) = region.wheel_pixels(scroll_region) else {
            return Ok(true);
        };
        self.stage_xui08_viewer_input(viewer).wheel(
            scroll_region,
            emitted_notches,
            step,
            region.maximum(scroll_region),
            region.smooth(scroll_region),
        )?;
        Ok(true)
    }

    pub fn xui08_pointer_press(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> Result<bool> {
        let Some(presented) = self.xui08_presented.as_ref() else {
            return Ok(false);
        };
        let Some(region) = presented.input_region.clone() else {
            return Ok(false);
        };
        let current =
            xui08_current_viewer_state(&self.xui08_pending_viewer_input, &viewer.peace_conference);
        if !current.open || !region.panel_contains((x, y)) {
            return Ok(false);
        }
        let staged_snapshot = self
            .xui08_pending_viewer_input
            .as_ref()
            .unwrap_or(&viewer.peace_conference)
            .clone();
        if let Some(action) = region.scrollbar_action_at(&staged_snapshot, (x, y))? {
            self.stage_xui08_viewer_input(viewer)
                .apply_scrollbar_action(&region, action)?;
            return Ok(true);
        }
        let target = presented
            .hit_targets
            .iter()
            .rev()
            .find(|target| target.contains((x, y)))
            .cloned();
        if let Some(target) = target {
            use game_ui::peace_conference_ui::{
                PeaceConferenceHistoryTab, PeaceConferenceHitAction,
            };
            let staged = self.stage_xui08_viewer_input(viewer);
            match &target.action {
                PeaceConferenceHitAction::SelectParticipant(id) => {
                    staged.selected_participant_id = Some(id.clone());
                    if !staged.expanded_participant_ids.remove(id) {
                        staged.expanded_participant_ids.insert(id.clone());
                    }
                }
                PeaceConferenceHitAction::SelectAllocation(id) => {
                    if staged.selected_allocation_id.as_deref() == Some(id) {
                        staged.clear_contested_popup();
                    } else {
                        staged.select_allocation(id.clone());
                    }
                }
                PeaceConferenceHitAction::ToggleTargetGroup(id) => {
                    if !staged.expanded_action_kinds.remove(id) {
                        staged.expanded_action_kinds.insert(id.clone());
                    }
                }
                PeaceConferenceHitAction::VictorsTab => {
                    staged.history_tab = PeaceConferenceHistoryTab::Victors;
                }
                PeaceConferenceHitAction::BeneficiariesTab => {
                    staged.history_tab = PeaceConferenceHistoryTab::Beneficiaries;
                }
                PeaceConferenceHitAction::ClosePanel => {
                    staged.set_open(false);
                    self.xui08_cursor = None;
                }
                PeaceConferenceHitAction::CloseContestedPopup => {
                    staged.clear_contested_popup();
                }
            }
        }
        Ok(true)
    }

    /// Toggle the state-first politics panel only from the last Presented
    /// passive carrier. Pending or withdrawn state cannot open a panel.
    ///
    /// This doc comment is a GATE INPUT, not decoration: the ui_runtime
    /// boundary tests slice this file at its first line to delimit the seek
    /// reset region. Do not delete it without re-anchoring those tests.
    pub fn toggle_xui11(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        if viewer.politics.panel_open {
            viewer.politics.close();
            return Ok(());
        }
        // A COMPARISON: there is no frame in an input handler, and the
        // presented carrier below carries a name its own frame admitted.
        if !viewer.selected_country.is_selected() {
            return Err("XUI11 cannot open without an observer-selected country".into());
        }
        let country = &viewer.selected_country;
        let presented = self
            .xui11_presented
            .as_ref()
            .filter(|presented| country.is(&presented.country_tag))
            .ok_or("XUI11 has no Presented passive carrier for the selected country")?;
        if presented.record_sha256.is_empty() {
            return Err("XUI11 Presented passive carrier identity is empty".into());
        }
        viewer.politics.toggle(Some(presented.country_tag.as_str()));
        Ok(())
    }

    pub fn xui11_wheel_notches_at(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
        emitted_notches: i32,
    ) -> Result<bool> {
        let Some(presented) = self.xui11_presented.as_ref() else {
            return Ok(false);
        };
        let Some(region) = presented.input_region else {
            return Ok(false);
        };
        if !viewer.politics.panel_open
            || !viewer.politics.country_tag.is(presented.country_tag.as_str())
            || !region.contains(x, y)
        {
            return Ok(false);
        }
        viewer.politics.wheel(
            emitted_notches,
            region.wheel_pixels,
            region.maximum_scroll_fixed,
            region.smooth,
        )?;
        Ok(true)
    }

    pub fn xui11_blocks_lower_pointer_input(&self, x: i32, y: i32) -> bool {
        self.xui11_presented
            .as_ref()
            .and_then(|presented| presented.input_region)
            .is_some_and(|region| region.panel_contains(x, y))
    }

    pub fn xui11_pointer_press(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> Result<bool> {
        let Some(presented) = self.xui11_presented.as_ref() else {
            return Ok(false);
        };
        let Some(region) = presented.input_region else {
            return Ok(false);
        };
        if !region.panel_contains(x, y)
            || !viewer.politics.panel_open
            || !viewer.politics.country_tag.is(presented.country_tag.as_str())
        {
            return Ok(false);
        }
        if region.close_contains(x, y) {
            viewer.politics.close();
            return Ok(true);
        }
        if region.power_balance_launcher_contains(x, y) {
            self.toggle_xui18(viewer)?;
            return Ok(true);
        }
        let Some(presentation) = presented.presentation.as_ref() else {
            return Ok(true);
        };
        if let Some(idea_id) = self
            .renderer
            .xui11_idea_at(
                &viewer.politics,
                presentation,
                (x, y),
                presented.viewport[0],
                presented.viewport[1],
            )
            .map_err(|error| format!("XUI11 idea hit admission refused: {error:?}"))?
        {
            viewer.politics.selected_idea_id = Some(idea_id);
        }
        Ok(true)
    }

    pub fn xui11_pointer_move(&mut self, x: i32, y: i32) {
        self.xui11_cursor = Some((x, y));
    }

    pub fn toggle_xui18(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        if viewer.power_balance.panel_open {
            viewer.power_balance.close();
            self.xui18_cursor = None;
            return Ok(());
        }
        // A COMPARISON: there is no frame in an input handler, and the
        // presented carrier below carries a name its own frame admitted.
        if !viewer.selected_country.is_selected() {
            return Err("XUI18 cannot open without an observer-selected country".into());
        }
        let country = &viewer.selected_country;
        let presented = self
            .xui18_presented
            .as_ref()
            .filter(|presented| country.is(&presented.country_tag) && presented.has_active_balance())
            .ok_or("XUI18 has no Presented active balance for the selected country")?;
        if presented.state_semantic_receipt_sha256.len() != 64 {
            return Err("XUI18 Presented state receipt is invalid".into());
        }
        viewer.power_balance.open();
        Ok(())
    }

    pub fn xui18_blocks_lower_pointer_input(
        &self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> bool {
        xui18_blocks_lower_pointer_input_for_state(
            viewer.power_balance.panel_open,
            &viewer.selected_country,
            self.xui18_presented.as_ref(),
            x,
            y,
        )
    }

    pub fn xui18_pointer_press(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> Result<bool> {
        if !viewer.power_balance.panel_open {
            return Ok(false);
        }
        let Some(presented) = self.xui18_presented.as_ref() else {
            return Ok(true);
        };
        if !presented.has_active_balance()
            || !viewer.selected_country.is(presented.country_tag.as_str())
        {
            return Ok(true);
        }
        let Some(panel) = presented.panel_region else {
            return Ok(true);
        };
        if !panel.contains(x, y) {
            return Ok(false);
        }
        if presented
            .close_region
            .is_some_and(|region| region.contains(x, y))
        {
            viewer.power_balance.close();
            self.xui18_cursor = None;
        }
        Ok(true)
    }

    pub fn xui18_pointer_move(&mut self, x: i32, y: i32) {
        self.xui18_cursor = Some((x, y));
    }

    pub fn toggle_xui09(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        if viewer.research.panel_open {
            viewer.research.close();
            self.xui09_drag_cursor = None;
            self.xui09_cursor = None;
            return Ok(());
        }
        // A COMPARISON: there is no frame in an input handler, and the
        // presented carrier below carries a name its own frame admitted.
        if !viewer.selected_country.is_selected() {
            return Err("XUI09 cannot open without an observer-selected country".into());
        }
        let country = &viewer.selected_country;
        let presented = self
            .xui09_presented
            .as_ref()
            .filter(|presented| country.is(&presented.country_tag))
            .ok_or(
                "XUI09 has no Presented complete-D18 research carrier for the selected country",
            )?;
        if presented.frame_hour != presented.model.world_hour
            || presented.available_folders.is_empty()
        {
            return Err("XUI09 Presented carrier or authored-folder roster is invalid".into());
        }
        if viewer
            .research
            .selected_folder
            .as_ref()
            .is_none_or(|folder| !presented.available_folders.iter().any(|row| row == folder))
        {
            viewer.research.selected_folder = presented.available_folders.first().cloned();
            viewer.research.pan_pixels = [0, 0];
            viewer.research.selected_technology_id = None;
        }
        self.xui09_drag_cursor = None;
        self.xui09_cursor = None;
        viewer.research.panel_open = true;
        Ok(())
    }

    pub fn close_xui09(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) {
        viewer.research.close();
        self.xui09_drag_cursor = None;
        self.xui09_cursor = None;
    }

    pub fn reset_xui09_for_country_change(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) {
        reset_xui09_country_bound_state(
            &mut viewer.research,
            &mut self.xui09_presented,
            &mut self.xui09_pending,
            &mut self.xui09_pending_staged,
            &mut self.xui09_pending_viewer_checkpoint,
            &mut self.xui09_drag_cursor,
            &mut self.xui09_cursor,
        );
    }

    pub fn cycle_xui09_folder(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        // A COMPARISON: there is no frame in an input handler, and the
        // presented carrier below carries a name its own frame admitted.
        if !viewer.selected_country.is_selected() {
            return Err("XUI09 cannot cycle folders without an observer-selected country".into());
        }
        let country = &viewer.selected_country;
        let presented = self
            .xui09_presented
            .as_ref()
            .filter(|presented| country.is(&presented.country_tag))
            .ok_or(
                "XUI09 has no Presented complete-D18 research carrier for the selected country",
            )?;
        if !viewer.research.panel_open || presented.available_folders.is_empty() {
            return Err("XUI09 folder cycling requires an open Presented research panel".into());
        }
        let current = viewer.research.selected_folder.as_deref();
        let next_index = current
            .and_then(|folder| {
                presented
                    .available_folders
                    .iter()
                    .position(|row| row == folder)
            })
            .map(|index| (index + 1) % presented.available_folders.len())
            .unwrap_or_default();
        viewer.research.selected_folder = Some(presented.available_folders[next_index].clone());
        viewer.research.pan_pixels = [0, 0];
        viewer.research.selected_technology_id = None;
        self.xui09_drag_cursor = None;
        Ok(())
    }

    pub fn xui09_blocks_lower_pointer_input(
        &self,
        viewer: &game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> bool {
        xui09_blocks_lower_pointer_input_for_state(
            viewer.research.panel_open,
            &viewer.selected_country,
            self.xui09_presented.as_ref(),
            x,
            y,
        )
    }

    pub fn xui09_pointer_press(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
        select_node: bool,
    ) -> Result<()> {
        if !viewer.research.panel_open {
            return Err("XUI09 pointer press is outside its Presented country surface".into());
        }
        let Some((presented, region)) = xui09_matching_presented_surface(
            viewer.research.panel_open,
            &viewer.selected_country,
            self.xui09_presented.as_ref(),
        ) else {
            self.xui09_drag_cursor = None;
            self.xui09_cursor = None;
            return Ok(());
        };
        if !region.contains(x, y) {
            return Err("XUI09 pointer press is outside its Presented country surface".into());
        }
        if select_node {
            viewer.research.selected_technology_id = presented
                .node_hit_targets
                .iter()
                .rev()
                .find(|target| target.contains(x, y))
                .map(|target| target.technology_id.clone());
        }
        self.xui09_cursor = Some((x, y));
        self.xui09_drag_cursor = Some([x, y]);
        Ok(())
    }

    pub fn xui09_pointer_move(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> Result<bool> {
        if xui09_matching_presented_surface(
            viewer.research.panel_open,
            &viewer.selected_country,
            self.xui09_presented.as_ref(),
        )
        .is_none()
        {
            self.xui09_drag_cursor = None;
            self.xui09_cursor = None;
            return Ok(false);
        }
        self.xui09_cursor = Some((x, y));
        let Some(previous) = self.xui09_drag_cursor else {
            return Ok(false);
        };
        self.xui09_drag_cursor = Some([x, y]);
        viewer.research.pan_by([x - previous[0], y - previous[1]])?;
        Ok(true)
    }

    pub fn xui09_pointer_release(&mut self) -> bool {
        self.xui09_drag_cursor.take().is_some()
    }

    pub fn xui09_capture_lost(&mut self) -> bool {
        let released = self.xui09_drag_cursor.take().is_some();
        if let Some(checkpoint) = self.observer_frame_checkpoint.as_mut() {
            checkpoint.xui09_drag_cursor = None;
        }
        released
    }

    pub fn open_xui10(
        &self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) -> Result<()> {
        if !viewer.selected_country.is_selected() {
            return Err("XUI10 cannot open without an observer-selected country".into());
        }
        viewer.focus.open();
        Ok(())
    }

    pub fn close_xui10(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    ) {
        viewer.focus.close();
        self.xui10_drag_cursor = None;
    }

    pub fn xui10_blocks_lower_pointer_input(&self, x: i32, y: i32) -> bool {
        self.xui10_presented
            .as_ref()
            .is_some_and(|presented| presented.input_region.contains(x, y))
    }

    pub fn xui10_pointer_press(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
        select_node: bool,
    ) -> Result<()> {
        let presented = self
            .xui10_presented
            .as_ref()
            .ok_or("XUI10 has no Presented authenticated frame")?;
        if !viewer.selected_country.is(presented.country_tag.as_str())
            || !presented.input_region.contains(x, y)
        {
            return Err("XUI10 pointer press is outside its Presented country surface".into());
        }
        if select_node {
            viewer.focus.selected_focus_id = presented
                .node_regions
                .iter()
                .rev()
                .find(|node| node.region.contains(x, y))
                .map(|node| node.focus_id.clone());
        }
        self.xui10_drag_cursor = Some([x, y]);
        Ok(())
    }

    pub fn xui10_pointer_move(
        &mut self,
        viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> Result<bool> {
        let Some(previous) = self.xui10_drag_cursor else {
            return Ok(false);
        };
        self.xui10_drag_cursor = Some([x, y]);
        viewer.focus.pan_by([x - previous[0], y - previous[1]])?;
        Ok(true)
    }

    pub fn xui10_pointer_release(&mut self) -> bool {
        self.xui10_drag_cursor.take().is_some()
    }

    pub fn xui10_presented_progress(
        &self,
    ) -> Option<&game_ui::focus_observer::ObserverFocusProgressBinding> {
        self.xui10_presented
            .as_ref()
            .and_then(|presented| presented.progress.as_ref())
    }

    pub fn xui23_blocks_lower_pointer_input(&self, x: i32, y: i32) -> bool {
        self.xui23_presented
            .as_ref()
            .and_then(|presented| presented.input_region)
            .is_some_and(|region| region.contains(x, y))
    }

    pub fn xui23_pointer_press(&self, x: i32, y: i32) -> Option<ObserverViewerActionDisposition> {
        self.xui23_presented.as_ref().and_then(|presented| {
            presented.button_regions.iter().find_map(|region| {
                region.contains(x, y).then(|| {
                    ObserverViewerActionDisposition::RefusedAbilityActivation {
                        ability_id: region.ability_id.clone(),
                        caster: region.caster.clone(),
                        group_target: region.group_target.clone(),
                    }
                })
            })
        })
    }

    pub fn dispatch_authenticated_viewer_action(
        &mut self,
        action: &game_ui::window_manager_law::AuthenticatedReplayViewerAction,
    ) -> Result<ObserverViewerActionDisposition> {
        use game_ui::window_manager_law::{
            MapmodeViewerAction, PoliticsViewerAction, ProductionLinesViewerAction,
            ReplayViewerAction, TheatreViewerAction, TransportViewerAction,
        };
        match action.action() {
            ReplayViewerAction::Transport(TransportViewerAction::SetPlaybackSpeedStep(step)) => {
                let consumed = self
                    .authored_observer_ui
                    .as_mut()
                    .ok_or("XUI02 authored observer UI is unavailable")?
                    .consume_authenticated_viewer_action(action)?;
                if &consumed != action.action() || !(1..=5).contains(step) {
                    return Err("authenticated transport action changed while consuming".into());
                }
                Ok(ObserverViewerActionDisposition::SetPlaybackSpeedStep(*step))
            }
            ReplayViewerAction::Mapmode(MapmodeViewerAction::SelectMapmodeButton(ordinal)) => {
                let consumed = self
                    .authored_observer_ui
                    .as_mut()
                    .ok_or("XUI02 authored observer UI is unavailable")?
                    .consume_authenticated_viewer_action(action)?;
                if &consumed != action.action() || !(1..=11).contains(ordinal) {
                    return Err("authenticated mapmode action changed while consuming".into());
                }
                Ok(ObserverViewerActionDisposition::SelectMapmodeButton(
                    *ordinal,
                ))
            }
            ReplayViewerAction::Politics(PoliticsViewerAction::ClosePolitics) => {
                let consumed = self
                    .authored_observer_ui
                    .as_mut()
                    .ok_or("XUI02 authored observer UI is unavailable")?
                    .consume_authenticated_viewer_action(action)?;
                if &consumed != action.action() {
                    return Err("authenticated viewer action changed while consuming".into());
                }
                self.close_authored_panel(action.window_id())?;
                Ok(ObserverViewerActionDisposition::Applied)
            }
            ReplayViewerAction::Politics(PoliticsViewerAction::OpenFocusTree) => {
                let consumed = self
                    .authored_observer_ui
                    .as_mut()
                    .ok_or("XUI02 authored observer UI is unavailable")?
                    .consume_authenticated_viewer_action(action)?;
                if &consumed != action.action() {
                    return Err("authenticated focus-tree action changed while consuming".into());
                }
                Ok(ObserverViewerActionDisposition::OpenFocusTree)
            }
            ReplayViewerAction::Politics(PoliticsViewerAction::OpenPowerBalance) => {
                let consumed = self
                    .authored_observer_ui
                    .as_mut()
                    .ok_or("XUI02 authored observer UI is unavailable")?
                    .consume_authenticated_viewer_action(action)?;
                if &consumed != action.action()
                    || !self
                        .xui18_presented
                        .as_ref()
                        .is_some_and(Xui18PresentedFrame::has_active_balance)
                {
                    return Err(
                        "authenticated power-balance action has no Presented XUI18 carrier".into(),
                    );
                }
                Ok(ObserverViewerActionDisposition::OpenPowerBalance)
            }
            ReplayViewerAction::Politics(PoliticsViewerAction::RefuseGameplayMutation(
                mutation,
            )) => {
                let consumed = self
                    .authored_observer_ui
                    .as_mut()
                    .ok_or("XUI02 authored observer UI is unavailable")?
                    .consume_authenticated_viewer_action(action)?;
                if &consumed != action.action() {
                    return Err("authenticated viewer action changed while consuming".into());
                }
                Ok(ObserverViewerActionDisposition::RefusedGameplayMutation(
                    mutation.clone(),
                ))
            }
            ReplayViewerAction::Politics(other) => Err(format!(
                "authenticated politics viewer action {other:?} has no installed typed panel-owner dispatcher"
            )
            .into()),
            ReplayViewerAction::Theatre(TheatreViewerAction::OpenLandSettings(object_id)) => {
                let consumed = self
                    .authored_observer_ui
                    .as_mut()
                    .ok_or("XUI02 authored observer UI is unavailable")?
                    .consume_authenticated_viewer_action(action)?;
                if &consumed != action.action() {
                    return Err("authenticated theatre action changed while consuming".into());
                }
                Ok(ObserverViewerActionDisposition::OpenLandTheatreSettings(
                    *object_id,
                ))
            }
            ReplayViewerAction::Theatre(TheatreViewerAction::CloseLandSettings(object_id)) => {
                let consumed = self
                    .authored_observer_ui
                    .as_mut()
                    .ok_or("XUI02 authored observer UI is unavailable")?
                    .consume_authenticated_viewer_action(action)?;
                if &consumed != action.action() {
                    return Err("authenticated theatre action changed while consuming".into());
                }
                Ok(ObserverViewerActionDisposition::CloseLandTheatreSettings(
                    *object_id,
                ))
            }
            ReplayViewerAction::ProductionLines(ProductionLinesViewerAction::Close) => {
                let consumed = self
                    .authored_observer_ui
                    .as_mut()
                    .ok_or("XUI02 authored observer UI is unavailable")?
                    .consume_authenticated_viewer_action(action)?;
                if &consumed != action.action() {
                    return Err("authenticated XUI12 close action changed while consuming".into());
                }
                self.close_authored_panel(action.window_id())?;
                Ok(ObserverViewerActionDisposition::CloseProductionLines)
            }
            ReplayViewerAction::ProductionLines(ProductionLinesViewerAction::ToggleRow(
                identity,
            )) => {
                let consumed = self
                    .authored_observer_ui
                    .as_mut()
                    .ok_or("XUI02 authored observer UI is unavailable")?
                    .consume_authenticated_viewer_action(action)?;
                if &consumed != action.action() {
                    return Err("authenticated XUI12 row action changed while consuming".into());
                }
                Ok(ObserverViewerActionDisposition::ToggleProductionLineRow(
                    identity.clone(),
                ))
            }
        }
    }
}


/// Performance-only selected-Present stress adapter.
///
/// This deliberately owns no semantic UI state and is never a replay or RE
/// authority. It reuses the exact production `ImageRenderer` upload/draw path
/// for a contract-pinned number of selected-Present-equivalent GPU operations.
/// The F1 receipt labels the result non-semantic and non-promotional.
///
/// GATE INPUT: the observer-runtime boundary test slices this file at this
/// block's first line; do not delete it without re-anchoring that test.
pub(crate) struct ProductSelectedPresentPerformanceRuntime {
    image: ImageRenderer,
}

impl ProductSelectedPresentPerformanceRuntime {
    pub(crate) fn new(gfx: &Gfx, surface: &game_ui::dds::RgbaImage) -> Result<Self> {
        let mut image = ImageRenderer::new(gfx)?;
        image.set_rgba(gfx, surface)?;
        Ok(Self { image })
    }

    pub(crate) fn execute(&self, gfx: &Gfx, operation_count: usize) -> Result<usize> {
        if operation_count == 0 {
            return Err("performance selected-Present operation count is zero".into());
        }
        for _ in 0..operation_count {
            self.image.draw(gfx)?;
        }
        Ok(operation_count)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{RawFrame, ReplaySource, SourceError, WorldTimeline};
    use serde_json::Map;

    struct FrameSource {
        hour: i64,
    }

    impl ReplaySource for FrameSource {
        fn game_id(&self) -> &str {
            "xui14-frame-identity"
        }

        fn hours(&self) -> &[i64] {
            std::slice::from_ref(&self.hour)
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
            Ok(RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: Map::new().into(),
            })
        }
    }

    fn test_world_frame(hour: i64) -> std::sync::Arc<WorldFrame> {
        WorldTimeline::open(Box::new(FrameSource { hour }))
            .expect("test timeline opens")
            .state_at(hour)
            .expect("test frame materializes")
    }

    /// A frame whose census says two D18 ids reach "DUP".
    ///
    /// replay_app had no contended fixture at all, so every contention refusal
    /// in this crate was untested and deleting one was a false green (Codex
    /// round 53, mutations 2 and 5).
    fn contended_test_world_frame(hour: i64) -> std::sync::Arc<WorldFrame> {
        struct ContendedSource {
            hour: i64,
        }
        impl ReplaySource for ContendedSource {
            fn game_id(&self) -> &str {
                "ub-contended-census"
            }
            fn hours(&self) -> &[i64] {
                std::slice::from_ref(&self.hour)
            }
            fn warnings(&self) -> &[String] {
                &[]
            }
            fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
                let mut rest = Map::new();
                rest.insert(
                    "countries".into(),
                    serde_json::json!({ "GER": { "_r2_tag_db_id": 12 } }),
                );
                rest.insert(
                    "tag_db_id_index".into(),
                    serde_json::json!({
                        "country_composition_version": 2,
                        "withheld_countries": {
                            "contended_keys": { "DUP": { "tag_db_ids": [441, 442] } }
                        },
                    }),
                );
                Ok(RawFrame {
                    hour_index,
                    date: None,
                    save_version: None,
                    schema_version: None,
                    rest: rest.into(),
                })
            }
        }
        WorldTimeline::open(Box::new(ContendedSource { hour }))
            .expect("contended test timeline opens")
            .state_at(hour)
            .expect("contended test frame materializes")
    }

    /// XUI19 does not close in silence on a key two ids reach.
    ///
    /// kills: the contention gap, and the ask that produces it.
    #[test]
    fn a_contended_xui19_selection_yields_no_candidate_and_names_why() {
        let frame = contended_test_world_frame(900);
        assert!(
            frame.country_key_is_contended("DUP"),
            "the census must be carried"
        );

        let mut gap = None;
        assert!(
            xui19_passive_candidate(&frame, &game_ui::stored_country_selection::StoredCountrySelection::of("DUP"), &mut gap).is_none(),
            "a key two ids reach is not a country to build a decision panel for"
        );
        let gap = gap.expect("the refusal must name its cause");
        assert!(
            gap.contains("contention_census") && gap.contains("DUP"),
            "the gap must name the census and the key: {gap}"
        );

        // THE CONTROL: a selection the census admits produces no contention gap
        // -- this fixture carries no decision carrier, so the candidate is
        // `None` for a different, named reason.
        let mut gap = None;
        assert!(xui19_passive_candidate(&frame, &game_ui::stored_country_selection::StoredCountrySelection::of("GER"), &mut gap).is_none());
        assert_eq!(
            gap, None,
            "an admitted selection is not a contention refusal, or this proves nothing"
        );
        let mut gap = None;
        assert!(xui19_passive_candidate(&frame, &game_ui::stored_country_selection::StoredCountrySelection::none(), &mut gap).is_none());
        assert_eq!(gap, None, "and neither is no selection at all");
    }

    /// The XUI16 outcome cache is keyed by the selector it was computed FOR.
    ///
    /// kills: folding a contended selection back into `None` on the way into
    /// the cache, which made one entry answer for two different selections.
    #[test]
    fn a_contended_selection_does_not_share_a_cache_entry_with_no_selection() {
        let frame = contended_test_world_frame(900);
        let runtime = ProductObserverOccupationRuntime {
            assembler: None,
            initialization_error: Some("cache key test".into()),
            last_outcome: Mutex::new(None),
        };
        let outcome = ObserverOccupationOutcome {
            surface: None,
            unavailable_reason: Some("cache key test".into()),
        };
        runtime.store_cached_outcome(&frame, &game_ui::stored_country_selection::StoredCountrySelection::of("DUP"), &outcome);

        assert_eq!(
            cached_occupation_outcome(&runtime.last_outcome, &frame, &game_ui::stored_country_selection::StoredCountrySelection::none()),
            None,
            "an outcome computed for a contended selection is not the outcome \
             for no selection at all"
        );
        assert_eq!(
            cached_occupation_outcome(&runtime.last_outcome, &frame, &game_ui::stored_country_selection::StoredCountrySelection::of("DUP")),
            Some(outcome),
            "and the entry still answers for the selection it was computed for, \
             or this proves nothing"
        );
    }

    fn xui19_presented(
        frame_hour: i64,
        source_sequence: u64,
        previous_record_sha256: Option<String>,
        record_sha256: String,
    ) -> Xui19PresentedFrame {
        Xui19PresentedFrame {
            country_tag: "GER".into(),
            frame_hour,
            source_sequence,
            previous_record_sha256,
            record_sha256,
            content_manifest_sha256: "cc".repeat(32),
            panel_drawn: true,
            panel_rect: Some([10, 20, 300, 400]),
            decision_grid_rect: Some([20, 60, 260, 330]),
            max_scroll_px: 90,
            category_extents: Vec::new(),
            card_hit_targets: Vec::new(),
            custom_icon_hit_targets: Vec::new(),
            provider_receipts: Vec::new(),
            xui22_widget_hit_targets: Vec::new(),
        }
    }

    #[test]
    fn xui19_history_accepts_only_identical_or_hash_linked_next_hour() {
        let hash = "aa".repeat(32);
        let first = xui19_presented(12, 40, Some("99".repeat(32)), hash.clone());
        let anchor = Xui19HistoryAnchor::from(&first);
        assert!(anchor.accepts(&first));
        assert!(!anchor.accepts(&xui19_presented(
            12,
            41,
            Some("99".repeat(32)),
            hash.clone(),
        )));
        assert!(anchor.accepts(&xui19_presented(
            13,
            41,
            Some(hash.clone()),
            "bb".repeat(32),
        )));
        assert!(!anchor.accepts(&xui19_presented(
            13,
            41,
            Some("ee".repeat(32)),
            "bb".repeat(32),
        )));
        assert!(!anchor.accepts(&xui19_presented(14, 42, Some(hash), "dd".repeat(32),)));
    }

    #[test]
    fn xui19_shipping_input_and_tooltips_are_presented_only_and_read_only() {
        let source = crate::source_scan::ui_runtime_production();
        // `ui_runtime_production()` already ends at the test module.
        let production = source.as_str();
        assert!(production.contains("self.xui19_presented = pending;"));
        assert!(
            production.contains("xui19_matching_presented(viewer, self.xui19_presented.as_ref())")
        );
        assert!(production.contains("xui19_card_tooltip_payloads("));
        assert!(production.contains("xui19_custom_icon_tooltip_payloads("));
        assert!(production.contains("xui22_widget_tooltip_payloads("));
        assert!(production.contains(
            "pending.xui22_widget_hit_targets = output.xui22_widget_hit_targets.clone();"
        ));
        assert!(production.contains(".xui22_widget_hit_targets"));
        assert!(production.contains("ObserverViewerActionDisposition::RefusedDecisionActivation"));
        assert!(production.contains(
            "ObserverViewerActionDisposition::RefusedDecisionCategoryProviderActivation"
        ));
        assert!(!production.contains("activate_decision("));
        assert!(!production.contains("apply_scripted_gui_effect("));
        assert!(xui19_rect_contains([10, 20, 30, 40], 10, 20));
        assert!(xui19_rect_contains([10, 20, 30, 40], 39, 59));
        assert!(!xui19_rect_contains([10, 20, 30, 40], 40, 59));
        assert!(!xui19_rect_contains([10, 20, 0, 40], 10, 20));

        let mut frame = xui19_presented(12, 40, None, "aa".repeat(32));
        let receipt = game_ui::production_renderer::ObserverDecisionCategoryProviderReceipt {
            category_id: "political_actions".into(),
            provider_id: "provider".into(),
            category_source_file: "common/decisions/categories/test.txt".into(),
            definition_source_file: "common/scripted_guis/test.txt".into(),
            authored_root_source_file: "interface/test.gui".into(),
            manager_order_index: 0,
            final_local_rect: [5, 10, 40, 50],
            stable_namespace: "xui19_category_0".into(),
            interaction: Default::default(),
        };
        assert_eq!(
            xui19_provider_screen_rect(&frame, &receipt),
            Some([25, 70, 40, 50])
        );
        frame.decision_grid_rect = Some([20, 60, 30, 30]);
        assert_eq!(
            xui19_provider_screen_rect(&frame, &receipt),
            Some([25, 70, 25, 20])
        );
    }

    #[test]
    fn xui20_shipping_route_is_present_transactional_and_history_bound() {
        let source = crate::source_scan::ui_runtime_production();
        // `ui_runtime_production()` already ends at the test module.
        let production = source.as_str();
        assert!(production
            .contains("self.xui20_pending_viewer_checkpoint = Some(viewer.combat.clone())"));
        assert!(production.contains("self.renderer.compose_xui20_combat_surface("));
        assert!(production.contains("self.xui20_history_anchor"));
        assert!(production
            .contains("self.xui20_history_anchor = Some(Xui20HistoryAnchor::from(presented))"));
        assert!(production.contains("fn rollback_xui20_staged_candidate("));
        assert!(production.contains("viewer.combat.reset_for_seek()"));
        assert!(production.contains("pub fn open_xui20_panel("));
        assert!(production.contains("pub fn xui20_wheel_notches_at("));
        assert!(production.contains("pub fn xui20_pointer_press("));
        assert!(production.contains("compose_xui20_xui05_tooltip("));
        let app = crate::source_scan::app_production();
        let execute_call = app
            .find("let outcome = match product_observer_ui.execute(")
            .expect("shipping observer execute match");
        let execute_tail = &app[execute_call..];
        assert!(execute_tail.contains("rollback_after_execute_error(&mut observer_viewer_ui)"));
        assert!(execute_tail.contains("return Err(error);"));
        let reject = production
            .find("pub fn reject_unpresented_authored_frame(")
            .expect("normal rejected-Present route");
        assert!(production[reject..].contains("rollback_xui20_staged_candidate("));
    }

    #[test]
    fn xui20_normal_hour_advance_retains_only_a_contiguous_present_panel_identity() {
        use game_ui::combat_detail_canvas::{Xui20ViewerPanel, Xui20ViewerState};

        let panel = Xui20ViewerPanel::LandCombat {
            combat_id: 44,
            generation: 3,
        };
        let candidate = Xui20PresentedFrame {
            frame_hour: 21,
            capture_sequence: 8,
            record_sha256: "aa".repeat(32),
            available_panels: vec![Xui20AvailablePanel {
                panel: panel.clone(),
                primary_scroll_max_px: 30,
                secondary_scroll_max_px: 12,
            }],
            input_region: None,
            tooltip_targets: Vec::new(),
        };
        let mut viewer = Xui20ViewerState::default();
        viewer.open(panel.clone(), 20);
        viewer.primary_scroll_px = 37;
        viewer.secondary_scroll_px = 19;
        reconcile_xui20_viewer_to_candidate(&mut viewer, Some(&candidate), 21);
        assert_eq!(viewer.panel, Some(panel));
        assert_eq!(viewer.selected_hour, Some(21));
        assert_eq!(viewer.primary_scroll_px, 30);
        assert_eq!(viewer.secondary_scroll_px, 12);

        reconcile_xui20_viewer_to_candidate(&mut viewer, Some(&candidate), 22);
        assert!(viewer.panel.is_none(), "noncontiguous candidate must close");
        assert_eq!(viewer.selected_hour, None);

        viewer.open(
            Xui20ViewerPanel::NavalLosses {
                country_tag: "ENG".into(),
            },
            21,
        );
        reconcile_xui20_viewer_to_candidate(&mut viewer, Some(&candidate), 21);
        assert!(viewer.panel.is_none(), "disappeared identity must close");
    }

    /// An open panel whose selection is contended costs THAT PANEL, not the
    /// frame.
    ///
    /// `execute` takes a live `&Gfx`, so there is no behavioural harness for it
    /// in this crate; the instrument the crate already uses for an
    /// `execute`-internal ordering invariant is a read of its own source
    /// (`xui20_execute_reconciles_the_authenticated_candidate_before_semantic_preflight`),
    /// and this is the same instrument. Codex round 53 turned each of these
    /// `break`s back into `return Err(..)` -- the whole-frame escalation this
    /// change was fixed to remove -- and all 642 replay_app tests stayed green.
    ///
    /// kills: escalating a contended selection out of its own panel.
    #[test]
    fn a_contended_open_panel_selection_costs_its_panel_and_not_the_frame() {
        let production = crate::source_scan::ui_runtime_production();
        let execute = production
            .find("    pub fn execute(
        &mut self,
        gfx: &Gfx,")
            .expect("product execute entry");
        let tail = &production[execute..];
        for label in ["xui09", "xui11", "xui14", "xui16", "xui18"] {
            let open = tail
                .find(&format!("'{label}: {{"))
                .unwrap_or_else(|| panic!("{label} open-panel block"));
            let block = &tail[open..];
            let gap = block
                .find("domain=contention_census:unavailable=CONTENDED_KEY (")
                .unwrap_or_else(|| panic!("{label} contention gap"));
            let out = block[gap..]
                .find(&format!("break '{label};"))
                .unwrap_or_else(|| {
                    panic!("{label} contention arm must leave its own block, not the frame")
                });
            let escalation = block[gap..gap + out].find("return Err");
            assert_eq!(
                escalation, None,
                "the {label} contention arm returns an error out of execute,                  which costs every other surface this frame"
            );
        }
    }

    #[test]
    fn xui20_execute_reconciles_the_authenticated_candidate_before_semantic_preflight() {
        let production = crate::source_scan::ui_runtime_production();
        let execute = production
            .find("    pub fn execute(\n        &mut self,\n        gfx: &Gfx,")
            .expect("product execute entry");
        let tail = &production[execute..];
        let reset = tail
            .find("self.reset_xui23_for_seek(viewer);")
            .expect("implicit seek reset in execute");
        let checkpoint = tail
            .find("self.xui20_pending_viewer_checkpoint = Some(viewer.combat.clone())")
            .expect("XUI20 current-frame viewer checkpoint");
        let reconcile = tail
            .find("reconcile_xui20_viewer_to_candidate(")
            .expect("XUI20 candidate reconciliation in execute");
        let staged = tail
            .find("self.xui20_pending_staged = true;")
            .expect("XUI20 transactional staging in execute");
        let semantic = tail
            .find(".prepare_observer_semantic_frame_with_xui12_and_xui22_producers(")
            .expect("semantic preflight in execute");
        assert!(
            reset < checkpoint && checkpoint < reconcile && reconcile < staged && staged < semantic
        );
        assert!(tail[..semantic]
            .contains("self.xui20_pending_viewer_checkpoint = Some(viewer.combat.clone())"));
        let execute_body = &tail[..tail
            .find("\n    pub fn invalidate(")
            .expect("product execute exit")];
        assert_eq!(
            execute_body
                .matches("rollback_xui20_staged_candidate(")
                .count(),
            2,
            "both initial semantic refusal and post-XUI15 reseal refusal must use the exact rollback"
        );
        assert!(execute_body.contains("observer UI semantic resealing refused"));
    }

    #[test]
    fn xui20_unpresented_rollback_restores_viewer_cursor_and_clears_staging() {
        use game_ui::combat_detail_canvas::{Xui20ViewerPanel, Xui20ViewerState};

        let panel = Xui20ViewerPanel::LandCombat {
            combat_id: 44,
            generation: 3,
        };
        let mut checkpoint = Xui20ViewerState::default();
        checkpoint.open(panel, 20);
        checkpoint.primary_scroll_px = 11;
        checkpoint.secondary_scroll_px = 7;
        let expected = checkpoint.clone();
        let mut viewer = Xui20ViewerState::default();
        let mut checkpoint = Some(checkpoint);
        let mut cursor = Some((91, 73));
        let expected_cursor = Some((17, 19));
        let mut cursor_checkpoint = expected_cursor;
        let mut pending = Some(Xui20PresentedFrame {
            frame_hour: 21,
            capture_sequence: 8,
            record_sha256: "aa".repeat(32),
            available_panels: Vec::new(),
            input_region: None,
            tooltip_targets: Vec::new(),
        });
        let mut staged = true;

        rollback_xui20_staged_candidate(
            &mut viewer,
            &mut checkpoint,
            &mut cursor,
            &mut cursor_checkpoint,
            &mut pending,
            &mut staged,
        )
        .expect("staged XUI20 refusal rollback");

        assert_eq!(viewer, expected);
        assert!(checkpoint.is_none());
        assert_eq!(cursor, expected_cursor);
        assert!(cursor_checkpoint.is_none());
        assert!(pending.is_none());
        assert!(!staged);
    }

    #[test]
    fn xui11_history_anchor_survives_unavailable_hours_and_refuses_restarts() {
        let anchor = Xui11HistoryAnchor {
            frame_hour: 20,
            capture_sequence: 7,
            record_sha256: "aa".repeat(32),
        };
        assert!(anchor.accepts(20, 7, Some(&"ff".repeat(32)), &"aa".repeat(32)));
        assert!(anchor.accepts(21, 8, Some(&"aa".repeat(32)), &"bb".repeat(32)));
        assert!(!anchor.accepts(22, 9, Some(&"bb".repeat(32)), &"cc".repeat(32)));
        assert!(!anchor.accepts(21, 8, Some(&"dd".repeat(32)), &"bb".repeat(32)));
        assert!(!anchor.accepts(21, 1, None, &"ee".repeat(32)));
    }

    #[test]
    fn xui14_history_and_present_lifecycle_are_atomic() {
        let anchor = Xui14HistoryAnchor {
            frame_hour: 20,
            capture_sequence: 7,
            record_sha256: "aa".repeat(32),
        };
        assert!(anchor.accepts(20, 7, Some(&"ff".repeat(32)), &"aa".repeat(32)));
        assert!(anchor.accepts(21, 8, Some(&"aa".repeat(32)), &"bb".repeat(32)));
        assert!(!anchor.accepts(22, 9, Some(&"bb".repeat(32)), &"cc".repeat(32)));
        assert!(!anchor.accepts(21, 8, Some(&"dd".repeat(32)), &"bb".repeat(32)));

        let source = production_source();
        let execute_start = source
            .find("    pub fn execute(\n        &mut self,\n        gfx: &Gfx,")
            .expect("product observer execute");
        let execute_end = source[execute_start..]
            .find("    pub fn open_authored_panel(")
            .map(|offset| execute_start + offset)
            .expect("product observer execute boundary");
        let execute = &source[execute_start..execute_end];
        assert!(execute
            .contains("self.xui14_pending_viewer_checkpoint = Some(viewer.intelligence.clone())"));
        assert!(execute.contains(".intelligence_presentation_exact()"));
        assert!(execute.contains("self.renderer.compose_xui14_intelligence_surface("));
        assert!(execute.contains("NON_CONTIGUOUS_OR_REWRITTEN_PASSIVE_RECORD"));
        assert!(execute.contains("HISTORY_REJECTED_CANDIDATE"));

        let confirm_start = source
            .find("    pub fn confirm_presented_authored_frame(&mut self)")
            .expect("present confirmation");
        let reject_start = source[confirm_start..]
            .find("    pub fn reject_unpresented_authored_frame(")
            .map(|offset| confirm_start + offset)
            .expect("present rejection");
        let confirm = &source[confirm_start..reject_start];
        assert!(confirm
            .contains("self.xui14_history_anchor = Some(Xui14HistoryAnchor::from(presented))"));
        let reject_end = source[reject_start..]
            .find("    pub fn open_xui23_context(")
            .map(|offset| reject_start + offset)
            .expect("present rejection boundary");
        let reject = &source[reject_start..reject_end];
        assert!(reject.contains("viewer.intelligence = self"));
        assert!(reject.contains(".xui14_pending_viewer_checkpoint"));
        assert!(!reject.contains("xui14_history_anchor = None"));

        let block_start = source
            .find("    pub fn xui14_blocks_lower_pointer_input(")
            .expect("XUI14 lower-layer pointer gate");
        let block_end = source[block_start..]
            .find("    pub fn xui14_pointer_press(")
            .map(|offset| block_start + offset)
            .expect("XUI14 lower-layer pointer gate boundary");
        let block = &source[block_start..block_end];
        let open_gate = block
            .find("viewer.intelligence.panel_open")
            .expect("current viewer-open gate");
        let stale_presented_lookup = block
            .find(".xui14_presented")
            .expect("Presented geometry lookup");
        assert!(open_gate < stale_presented_lookup);

        let wheel_start = source
            .find("    pub fn xui14_wheel_notches_at(")
            .expect("XUI14 wheel route");
        let wheel_end = source[wheel_start..]
            .find("    pub fn xui14_pointer_move(")
            .map(|offset| wheel_start + offset)
            .expect("XUI14 wheel route boundary");
        let wheel = &source[wheel_start..wheel_end];
        let pageable_gate = wheel
            .find("region.row_count <= 1")
            .expect("non-pageable XUI14 wheel refusal");
        let panel_hit_gate = wheel
            .find("region.panel_contains(x, y)")
            .expect("XUI14 panel hit gate");
        assert!(pageable_gate < panel_hit_gate);
    }

    #[test]
    fn xui11_present_disposition_owns_history_and_viewer_rollback() {
        let source = production_source();
        let execute_start = source
            .find("    pub fn execute(\n        &mut self,\n        gfx: &Gfx,")
            .expect("product observer execute");
        let execute_end = source[execute_start..]
            .find("    pub fn open_authored_panel(")
            .map(|offset| execute_start + offset)
            .expect("product observer execute boundary");
        let execute = &source[execute_start..execute_end];
        let checkpoint = execute
            .find("self.xui11_pending_viewer_checkpoint = Some(viewer.politics.clone())")
            .expect("XUI11 viewer checkpoint");
        let advance = execute
            .find(".advance_smooth_scroll(input.smooth_factor_fixed)")
            .expect("XUI11 smooth advance");
        assert!(checkpoint < advance);
        assert!(execute.contains(".xui11_history_anchor"));
        assert!(!execute.contains("xui11_presented.as_ref().is_none_or"));
        let history_gate = execute
            .find("if self.xui11_pending.is_some() {")
            .expect("history-approved XUI11 compositor gate");
        let compose = execute
            .find("self.renderer.compose_xui11_politics_surface(")
            .expect("XUI11 compositor call");
        assert!(history_gate < compose);
        assert!(execute[compose..].contains("HISTORY_REJECTED_CANDIDATE"));

        let confirm_start = source
            .find("    pub fn confirm_presented_authored_frame(&mut self)")
            .expect("present confirmation");
        let reject_start = source[confirm_start..]
            .find("    pub fn reject_unpresented_authored_frame(")
            .map(|offset| confirm_start + offset)
            .expect("present rejection");
        let confirm = &source[confirm_start..reject_start];
        assert!(confirm
            .contains("self.xui11_history_anchor = Some(Xui11HistoryAnchor::from(presented))"));
        assert!(confirm.contains("self.xui11_pending_viewer_checkpoint = None"));

        let reject_end = source[reject_start..]
            .find("    pub fn open_xui23_context(")
            .map(|offset| reject_start + offset)
            .expect("present rejection boundary");
        let reject = &source[reject_start..reject_end];
        assert!(reject.contains("viewer.politics = self"));
        assert!(reject.contains(".xui11_pending_viewer_checkpoint"));
        assert!(!reject.contains("xui11_history_anchor = None"));

        let invalidate_start = source
            .find("    pub fn invalidate(&mut self)")
            .expect("resize invalidation");
        let invalidate_end = source[invalidate_start..]
            .find("    pub fn designer_effective_membership(")
            .map(|offset| invalidate_start + offset)
            .expect("resize invalidation boundary");
        assert!(!source[invalidate_start..invalidate_end].contains("xui11_history_anchor = None"));

        let country_reset_start = source
            .find("    pub fn reset_country_dependent_ui_for_country_change(")
            .expect("country-bound reset");
        let reset_start = source[country_reset_start..]
            .find("    pub fn reset_xui23_for_seek(")
            .map(|offset| country_reset_start + offset)
            .expect("seek reset");
        let country_reset = &source[country_reset_start..reset_start];
        assert!(country_reset.contains("viewer.politics.reset_frame_bound_state()"));
        assert!(country_reset.contains("self.xui11_history_anchor = None"));
        let reset_end = source[reset_start..]
            .find("    pub fn toggle_xui11(")
            .map(|offset| reset_start + offset)
            .expect("seek reset boundary");
        let reset = &source[reset_start..reset_end];
        assert!(reset.contains("self.reset_country_dependent_ui_for_country_change(viewer)"));
    }

    #[test]
    fn xui18_shipping_route_is_present_transactional_and_viewer_owned() {
        let source = crate::source_scan::ui_runtime_production();
        // `ui_runtime_production()` already ends at the test module.
        let production = source.as_str();
        assert!(production.contains("compose_xui18_power_balance_surface("));
        assert!(production.contains("self.xui18_presented = self.xui18_pending.take()"));
        assert!(production.contains("self.xui18_pending_viewer_checkpoint = None"));
        assert!(production.contains("self.present_transaction.reject(viewer)"));
        assert!(production.contains("viewer.power_balance.reset_for_seek_or_country_change()"));
        assert!(production.contains("viewer.power_balance.panel_open"));
        assert!(production.contains(
            "viewer.selected_country.is(presented.country_tag.as_str())"
        ));
        assert!(production.contains("let xui18_carrier_candidate"));
        assert!(production.contains("admit_country_power_balance_ui("));
        assert!(production.contains("Xui18PresentedAvailability::MeasuredEmpty"));
        assert!(production.contains("viewer-closed:measured-empty"));
        assert!(production.contains("self.renderer.content_manifest_digest_sha256()"));
        assert!(production.contains("compose_packet_xui05_tooltip("));
        assert!(production.contains("\"XUI18\""));
        assert!(production.contains("region.power_balance_launcher_contains(x, y)"));
        assert!(production.contains("self.toggle_xui18(viewer)?"));
        assert!(production.contains("authenticated open panel produced no pixel surface"));
        assert!(production.contains("authenticated compositor rejected the exact surface"));

        let app = crate::source_scan::app_production();
        assert!(app.contains("ObserverViewerActionDisposition::OpenPowerBalance"));
        assert!(app.contains("xui18_pointer_press("));
        assert!(app.contains("xui18_pointer_move("));
        assert!(app.matches("xui18_blocks_lower_pointer_input(").count() >= 4);
        assert!(app.contains("&& !xui18_blocks_lower_input"));
        let xui11_wheel_gate = app
            .split_once("let xui11_consumed =")
            .expect("XUI11 wheel gate")
            .1
            .split_once("let xui14_blocks_lower_input")
            .expect("end of XUI11 wheel gate")
            .0;
        assert!(xui11_wheel_gate.contains("!xui18_blocks_lower_input"));
        assert!(app.contains("viewer.power_balance.reset_for_seek_or_country_change()"));
        assert!(app.contains("observer_viewer_ui.power_balance.close()"));
    }

    #[test]
    fn xui21_shipping_route_is_present_transactional_and_read_only() {
        let source = crate::source_scan::ui_runtime_production();
        // `ui_runtime_production()` already ends at the test module.
        let production = source.as_str();
        assert!(production.contains("compose_xui21_event_stack("));
        assert!(production.contains("begin_xui21_staged_candidate("));
        assert!(production.contains("stage_xui21_candidate("));
        assert!(production.contains("commit_xui21_staged_candidate("));
        assert!(production.contains("rollback_xui21_staged_candidate("));
        assert!(production.contains("event_pixels_available"));
        assert!(production.contains("viewer.events.reset_for_seek_or_country_change()"));
        assert!(production.contains("pub fn rollback_after_execute_error("));
        assert!(production.contains("pub fn reject_unpresented_authored_frame("));
        assert!(production.contains("pub fn xui21_blocks_lower_pointer_input("));
        assert!(production.contains("pub fn xui21_pointer_press("));
        assert!(production.contains("pub fn cycle_xui21_focus("));
        assert!(production.contains("pub fn dismiss_xui21_focused("));
        assert!(production.contains("RefusedEventOptionActivation"));
        assert!(production.contains("fn xui21_matching_presented_frame"));
        assert!(production.contains("selected_country.is(frame.country_tag.as_str())"));
        let pointer_start = production
            .find("    pub fn xui21_pointer_press(")
            .expect("XUI21 Presented pointer route");
        let pointer_end = production[pointer_start..]
            .find("    pub fn cycle_xui21_focus(")
            .map(|offset| pointer_start + offset)
            .expect("XUI21 pointer route boundary");
        let pointer = &production[pointer_start..pointer_end];
        assert!(pointer.contains(".xui21_presented"));
        assert!(!pointer.contains(".xui21_pending"));
    }

    fn xui21_test_popup(sequence: i64, x: i32) -> Xui21PresentedPopup {
        Xui21PresentedPopup {
            event_sequence: sequence,
            root_source: "interface/eventwindow.gui".into(),
            root_window: "country_event".into(),
            root_region: game_ui::event_chrome::EventPopupInputRegion {
                x,
                y: 0,
                width: 100,
                height: 100,
            },
            minimize_region: game_ui::event_chrome::EventPopupInputRegion {
                x: x + 80,
                y: 0,
                width: 20,
                height: 20,
            },
            minimize_child_path: vec![3, 1],
            minimize_tooltip_key: Some("CLOSE".into()),
            option_targets: vec![game_ui::event_chrome::EventOptionHitTarget {
                event_sequence: sequence,
                definition_ordinal: 4,
                region: game_ui::event_chrome::EventPopupInputRegion {
                    x: x + 10,
                    y: 40,
                    width: 30,
                    height: 20,
                },
            }],
        }
    }

    fn xui21_test_frame() -> Xui21PresentedFrame {
        Xui21PresentedFrame {
            country_tag: "ENG".into(),
            visible_sequences: vec![11, 12],
            focused_sequence: Some(12),
            popups: vec![xui21_test_popup(11, 0), xui21_test_popup(12, 100)],
        }
    }

    #[test]
    fn xui21_only_a_nonempty_composited_pixel_stack_can_become_presented() {
        let country = "ENG".to_string();
        assert!(xui21_staged_candidate(
            Some(&country),
            Some(900),
            true,
            false,
            &[11],
            Some(11),
            vec![xui21_test_popup(11, 0)],
        )
        .is_none());
        assert!(xui21_staged_candidate(
            Some(&country),
            Some(900),
            true,
            true,
            &[],
            None,
            Vec::new(),
        )
        .is_none());
        let presented = xui21_staged_candidate(
            Some(&country),
            Some(900),
            true,
            true,
            &[11],
            Some(11),
            vec![xui21_test_popup(11, 0)],
        )
        .expect("nonempty authenticated pixels stage");
        assert!(xui21_matching_presented_frame(&game_ui::stored_country_selection::StoredCountrySelection::of("ENG"), Some(&presented)).is_some());
        assert!(xui21_matching_presented_frame(&game_ui::stored_country_selection::StoredCountrySelection::of("FRA"), Some(&presented)).is_none());

        let focused_reordered = Xui21PresentedFrame {
            country_tag: country,
            visible_sequences: vec![11, 12],
            focused_sequence: Some(11),
            popups: vec![xui21_test_popup(12, 100), xui21_test_popup(11, 0)],
        };
        assert!(
            xui21_presented_frame_is_interactive(&focused_reordered),
            "production compositor moves the focused popup last without changing membership"
        );

        let no_focus = Xui21PresentedFrame {
            focused_sequence: None,
            ..focused_reordered.clone()
        };
        assert!(
            !xui21_presented_frame_is_interactive(&no_focus),
            "a Presented stack without one authenticated focus must fail closed"
        );

        let focused_below_pointer_topmost = Xui21PresentedFrame {
            focused_sequence: Some(12),
            ..focused_reordered
        };
        assert!(
            !xui21_presented_frame_is_interactive(&focused_below_pointer_topmost),
            "keyboard focus must equal the reverse-z pointer topmost popup"
        );
    }

    #[test]
    fn xui21_present_transaction_commit_and_both_rollback_paths_are_exact() {
        let mut viewer = game_ui::event_chrome::EventPopupViewerState::default();
        viewer.focused_event_sequence = Some(7);
        viewer.dismissed_event_sequences.insert(2);
        let expected = viewer.clone();
        let mut checkpoint = None;
        let mut pending = None;
        let mut staged = false;

        begin_xui21_staged_candidate(&viewer, &mut checkpoint, &pending, staged)
            .expect("begin rejected/error transaction");
        viewer.focused_event_sequence = Some(12);
        viewer.dismissed_event_sequences.insert(9);
        stage_xui21_candidate(
            &checkpoint,
            &mut pending,
            &mut staged,
            Some(xui21_test_frame()),
        )
        .expect("stage rejected/error transaction");
        rollback_xui21_staged_candidate(&mut viewer, &mut checkpoint, &mut pending, &mut staged)
            .expect("shared execute-error/rejected-Present rollback");
        assert_eq!(viewer, expected);
        assert!(checkpoint.is_none());
        assert!(pending.is_none());
        assert!(!staged);

        begin_xui21_staged_candidate(&viewer, &mut checkpoint, &pending, staged)
            .expect("begin confirmed transaction");
        stage_xui21_candidate(
            &checkpoint,
            &mut pending,
            &mut staged,
            Some(xui21_test_frame()),
        )
        .expect("stage confirmed transaction");
        let committed = commit_xui21_staged_candidate(&mut checkpoint, &mut pending, &mut staged)
            .expect("confirm Presented transaction");
        assert!(committed.is_some());
        assert!(checkpoint.is_none());
        assert!(pending.is_none());
        assert!(!staged);
    }

    #[test]
    fn xui21_presented_keyboard_and_pointer_input_use_only_drawn_popups() {
        let frame = xui21_test_frame();
        let mut viewer = game_ui::event_chrome::EventPopupViewerState::default();
        viewer.focused_event_sequence = Some(11);
        cycle_xui21_presented_focus(&frame, &mut viewer, false).expect("forward cycle");
        assert_eq!(viewer.focused_event_sequence, Some(12));
        cycle_xui21_presented_focus(&frame, &mut viewer, true).expect("backward cycle");
        assert_eq!(viewer.focused_event_sequence, Some(11));

        let option = press_xui21_presented_popup(&frame, &mut viewer, 20, 50)
            .expect("drawn option hit target");
        assert_eq!(
            option,
            ObserverViewerActionDisposition::RefusedEventOptionActivation {
                event_sequence: 11,
                definition_ordinal: 4,
            }
        );
        let minimize = press_xui21_presented_popup(&frame, &mut viewer, 190, 10)
            .expect("drawn minimize hit target");
        assert_eq!(minimize, ObserverViewerActionDisposition::Applied);
        assert!(viewer.dismissed_event_sequences.contains(&12));

        let malformed = Xui21PresentedFrame {
            country_tag: "ENG".into(),
            visible_sequences: Vec::new(),
            focused_sequence: None,
            popups: Vec::new(),
        };
        assert!(cycle_xui21_presented_focus(&malformed, &mut viewer, false).is_err());
        assert!(dismiss_xui21_presented_focus(&malformed, &mut viewer).is_err());
        assert!(press_xui21_presented_popup(&malformed, &mut viewer, 0, 0).is_err());
    }

    fn xui18_test_presented_frame(
        availability: Xui18PresentedAvailability,
        panel_region: Option<game_ui::power_balance_compositor::PowerBalanceInputRegion>,
    ) -> Xui18PresentedFrame {
        Xui18PresentedFrame {
            country_tag: "ITA".into(),
            frame_hour: 900,
            state_semantic_receipt_sha256: "ab".repeat(32),
            availability,
            panel_region,
            close_region: None,
            tooltip_targets: Vec::new(),
        }
    }

    #[test]
    fn xui18_initial_open_without_presented_geometry_absorbs_lower_pointer_input() {
        let presented = xui18_test_presented_frame(Xui18PresentedAvailability::Visible, None);
        assert!(xui18_blocks_lower_pointer_input_for_state(
            true,
            &game_ui::stored_country_selection::StoredCountrySelection::of("ITA"),
            Some(&presented),
            i32::MAX,
            i32::MIN,
        ));
        assert!(xui18_blocks_lower_pointer_input_for_state(
            true,
            &game_ui::stored_country_selection::StoredCountrySelection::of("ENG"),
            Some(&presented),
            0,
            0,
        ));
        assert!(!xui18_blocks_lower_pointer_input_for_state(
            false,
            &game_ui::stored_country_selection::StoredCountrySelection::of("ITA"),
            Some(&presented),
            0,
            0,
        ));
    }

    #[test]
    fn xui18_presented_measured_empty_is_not_openable_or_available() {
        let presented = xui18_test_presented_frame(
            Xui18PresentedAvailability::MeasuredEmpty,
            Some(game_ui::power_balance_compositor::PowerBalanceInputRegion {
                x: 10,
                y: 20,
                width: 30,
                height: 40,
            }),
        );
        assert!(!presented.has_active_balance());
        assert!(xui18_matching_presented_region(true, &game_ui::stored_country_selection::StoredCountrySelection::of("ITA"), Some(&presented)).is_none());
    }

    fn production_source() -> String {
        crate::source_scan::ui_runtime_production()
    }

    #[test]
    fn the_production_source_slice_actually_excludes_the_tests() {
        let production = production_source();
        // Against the STRIPPED whole file: the raw reader is private to the
        // seam, and the cut is a prefix of what the seam hands out, so this is
        // the comparison that says the cut removed something.
        assert!(production.len() < crate::source_scan::ui_runtime().len());
        assert!(!production.contains('\r'));
        let test_only = [
            "the_production_source_slice_",
            "actually_excludes_the_tests",
        ]
        .concat();
        assert!(!production.contains(&test_only));
        assert!(production.contains("fn cached_occupation_outcome("));
    }

    #[test]
    fn xui23_shipping_route_owns_frame_decode_present_and_open_seek_input() {
        let runtime = production_source();
        assert!(
            runtime.contains(".prepare_observer_semantic_frame_with_xui12_and_xui22_producers(")
        );
        assert!(!runtime.contains("production_lines_inputs.abilities"));
        assert!(runtime.contains("self.xui23_pending = Some(Xui23PresentedFrame"));
        assert!(runtime.contains("self.xui23_presented = self.xui23_pending.take()"));
        assert!(runtime.contains("pub fn cycle_xui23_context("));
        assert!(runtime.contains("pub fn reset_xui23_for_seek("));
        assert!(runtime
            .contains("if !viewer.abilities.open || viewer.abilities.selected_context.is_none()"));

        let execute_start = runtime
            .find("pub fn execute(\n        &mut self,\n        gfx: &Gfx,\n        world_frame:")
            .expect("shipping ProductObserverUiRuntime execute");
        let execute_end = runtime[execute_start..]
            .find("    pub fn invalidate(&mut self)")
            .map(|offset| execute_start + offset)
            .expect("shipping ProductObserverUiRuntime execute boundary");
        let execute = &runtime[execute_start..execute_end];
        let pending_refusal = execute
            .find("if self.xui23_pending.is_some()")
            .expect("pending-frame refusal");
        let automatic_reset = execute
            .find("self.reset_xui23_for_seek(viewer)")
            .expect("automatic discontinuity reset");
        assert!(pending_refusal < automatic_reset);

        let reset_start = runtime
            .find("pub fn reset_xui23_for_seek(")
            .expect("shipping XUI23 reset");
        let reset_end = runtime[reset_start..]
            .find("    pub fn xui23_blocks_lower_pointer_input")
            .map(|offset| reset_start + offset)
            .expect("shipping XUI23 reset boundary");
        assert!(!runtime[reset_start..reset_end].contains("self.xui23_pending = None"));

        let app = crate::source_scan::app_production();
        assert!(app.contains("VK_X if pov.selection().is_selected()"));
        assert!(app.contains("product_observer_ui.cycle_xui23_context"));
        for seek in [
            "transport.step_backward();",
            "transport.step_forward();",
            "transport.seek_to_index(tick.index);",
            "transport.seek_to_fraction(f64::from(fraction));",
        ] {
            let start = app.find(seek).expect("shipping seek path");
            let tail = &app[start..app.len().min(start + 300)];
            assert!(
                tail.contains("reset_xui23_for_seek"),
                "{seek} must invalidate XUI23 selection and animation"
            );
        }
    }

    #[test]
    fn observer_product_runtime_has_no_recorder_presentation_input() {
        let source = production_source();
        let start = source
            .find("impl ProductObserverUiRuntime {")
            .expect("observer product runtime");
        let end = source[start..]
            .find("pub(crate) struct ProductSelectedPresentPerformanceRuntime {")
            .map(|offset| start + offset)
            .expect("observer runtime boundary");
        let observer = &source[start..end];
        assert!(
            observer.contains(".prepare_observer_semantic_frame_with_xui12_and_xui22_producers(")
        );
        assert!(observer.contains(".bind_persistent_hud_roots("));
        assert!(observer.contains(".compose_open_panels_with_feature_local_retry("));
        // A feature-local retry names the feature that WENT: an XUI26 frontend
        // panel the compositor refused is not an XUI12 unavailability, and the
        // XUI12 gap is emitted only when production lines were the drop.
        let retry = observer
            .find("let (panels, late_feature_local_rejection)")
            .expect("the observer composes through the feature-local retry");
        let retry_end = observer[retry..]
            .find("authored_panel_draw_count")
            .map(|offset| retry + offset)
            .expect("feature-local retry handling boundary");
        let region = &observer[retry..retry_end];
        let branch = region
            .find("if rejection.production_lines.is_some() {")
            .expect("the XUI12 flags are conditional on XUI12 having gone");
        let branch_end = region[branch..]
            .find("
                }")
            .map(|offset| branch + offset)
            .expect("XUI12 flag branch boundary");
        for flag in [
            "production_lines_composited = false;",
            "production_lines_available = false;",
        ] {
            assert_eq!(
                region.matches(flag).count(),
                1,
                "{flag} is set once in the retry handling, inside the XUI12 branch"
            );
            assert!(region[branch..branch_end].contains(flag), "{flag}");
        }
        // And the rejection is CONSUMED: the handler is entered from the retry
        // call's own binding, and both halves of every gap it owes are pushed
        // inside the loop that produces them.
        assert_eq!(
            region
                .matches("if let Some(rejection) = late_feature_local_rejection {")
                .count(),
            1
        );
        let loop_start = region
            .find("for (line, gap) in rejection.observer_ui_gaps() {")
            .expect("every owed gap is emitted");
        let loop_end = region[loop_start..]
            .find("
                }")
            .map(|offset| loop_start + offset)
            .expect("gap loop boundary");
        for push in ["feature_local_gaps.push(line);", "output_gaps.push(gap);"] {
            assert!(region[loop_start..loop_end].contains(push), "{push}");
            assert_eq!(region.matches(push).count(), 1, "{push}");
        }
        assert!(observer.contains(".compose_xui15_trade_surface("));
        assert!(observer.contains("apply_xui15_feature_local_composition("));
        assert!(observer.contains("xui15_pointer_press("));
        assert!(observer.contains("xui15_wheel_notches_at("));
        assert!(observer.contains("xui15_blocks_lower_pointer_input("));
        assert!(!observer.contains(".compose_open_panels_into("));
        assert!(observer.contains("PROMOTED_EXTENSION_PACKETS"));
        for forbidden in [
            "AuthenticatedUiPass",
            "recorded_ui_owner",
            "resolved_tooltip_exact",
            "market_trade_activation",
            "hud_surface_activation",
            "active_mapmode_state",
        ] {
            assert!(
                !observer.contains(forbidden),
                "observer product runtime must not consume {forbidden}"
            );
        }
    }

    #[test]
    fn xui05_staged_geometry_and_controller_state_share_present_disposition() {
        let source = production_source();
        let execute_start = source
            .find("    pub fn execute(\n        &mut self,\n        gfx: &Gfx,")
            .expect("product observer execute");
        let execute_end = source[execute_start..]
            .find("    pub fn open_authored_panel(")
            .map(|offset| execute_start + offset)
            .expect("product observer execute boundary");
        let execute = &source[execute_start..execute_end];
        let panel_stage = execute
            .find(".compose_open_panels_with_feature_local_retry(")
            .expect("authenticated panel staging");
        let pending_context = execute
            .find(".authenticated_pending_tooltip_context_at_cursor(")
            .expect("same-frame pending tooltip context");
        let checkpoint = execute
            .find("self.xui05_pending_controller_checkpoint = Some(self.xui05_controller.clone())")
            .expect("XUI05 controller checkpoint");
        let compose = execute
            .find("compose_authenticated_xui05_tooltip(")
            .expect("XUI05 product composition");
        assert!(
            panel_stage < pending_context && pending_context < checkpoint && checkpoint < compose
        );
        assert!(execute.contains("self.reject_unpresented_authored_frame(viewer)"));

        let confirm_start = source
            .find("    pub fn confirm_presented_authored_frame(&mut self)")
            .expect("present confirmation");
        let reject_start = source[confirm_start..]
            .find("    pub fn reject_unpresented_authored_frame(")
            .map(|offset| confirm_start + offset)
            .expect("present rejection");
        let confirm = &source[confirm_start..reject_start];
        assert!(confirm.contains("current.as_ref() == Some(&owner)"));
        assert!(confirm.contains("self.xui05_pending_controller_checkpoint = None;"));

        let reject_end = source[reject_start..]
            .find("    pub fn dispatch_authenticated_viewer_action(")
            .map(|offset| reject_start + offset)
            .expect("present rejection boundary");
        let reject = &source[reject_start..reject_end];
        let manager_rollback = reject
            .find("authored.reject_unpresented_frame()?")
            .expect("manager rollback");
        let controller_rollback = reject
            .find("self.xui05_controller = self")
            .expect("controller rollback");
        assert!(manager_rollback < controller_rollback);
        assert!(reject.contains(".xui05_pending_controller_checkpoint"));
    }

    #[test]
    fn xui13_success_retains_pixels_and_forwards_each_named_field_or_subtree_refusal() {
        let mut framebuffer = game_ui::font::blank_image(2, 1);
        let mut image = game_ui::font::blank_image(2, 1);
        image.pixels.copy_from_slice(&[11,22,33,255,44,55,66,255]);
        let surface = game_ui::designer_observer::ObserverDesignerSurface {
            family: game_ui::readonly_design_peace::DesignerViewKind::Tank,
            owner_tag: "GER".into(), authored_root: "tank_designer_view".into(), source_files: vec![],
            ordered_design_ids: vec!["7:70".into()], visible_design_ids: vec!["7:70".into()], selected_design_id: None,
            ordered_country_filter_tags: vec![], visible_country_filter_tags: vec![], selected_country_filter_tags: vec![],
            output: game_ui::compose::ComposeOutput { image: image.clone(), report: Default::default(), debug_rects: vec![] },
            field_refusals: std::collections::BTreeMap::from([
                ("7:70/missions".into(), "missing exact mission operands".into()),
                ("row_suffix".into(), "ITA eligibility unavailable".into())]),
            membership_complete: false, country_filters_complete: false,
        };
        let mut count=3; let mut receipts=Vec::new(); let mut gaps=Vec::new(); let mut feature_gaps=Vec::new();
        assert!(apply_xui13_feature_local_composition(Ok(surface), &mut framebuffer, "GER", true,
            &mut count, &mut receipts, &mut gaps, &mut feature_gaps).unwrap());
        assert_eq!(framebuffer,image); assert_eq!(count,4); assert_eq!(receipts.len(),1);
        assert!(receipts[0].contains("measured_empty=false"));
        assert_eq!(gaps.len(),2); assert_eq!(feature_gaps.len(),2);
        assert!(gaps.iter().all(|gap|gap.surface=="XUI13/designer"));
        assert!(gaps[0].reason.contains("7:70/missions"));
        assert!(feature_gaps[1].contains("row_suffix"));
    }

    #[test]
    fn xui13_feature_local_rejection_is_atomic_and_reports_unavailable() {
        let mut framebuffer = game_ui::font::blank_image(2, 1);
        framebuffer
            .pixels
            .copy_from_slice(&[11, 22, 33, 255, 44, 55, 66, 255]);
        let before = framebuffer.clone();
        let mut draw_count = 3;
        let mut receipts = Vec::new();
        let mut gaps = Vec::new();
        let mut feature_gaps = Vec::new();
        let available = apply_xui13_feature_local_composition(
            Err(
                game_ui::production_renderer::ProductionUiRejection::IncompleteComposition(
                    "fixture missing admitted effective designer-window source".into(),
                ),
            ),
            &mut framebuffer,
            "GER",
            false,
            &mut draw_count,
            &mut receipts,
            &mut gaps,
            &mut feature_gaps,
        )
        .unwrap();

        assert!(!available);
        assert_eq!(
            framebuffer, before,
            "a rejected surface must contribute no pixels"
        );
        assert_eq!(draw_count, 3);
        assert!(receipts.is_empty());
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].surface, "XUI13/designer");
        assert!(gaps[0].reason.contains("designer-window source"));
        assert_eq!(feature_gaps.len(), 1);
        assert!(feature_gaps[0].contains("XUI13:field="));
    }

    #[test]
    fn xui16_outcome_cache_is_scoped_to_immutable_frame_identity_and_country() {
        let frame = test_world_frame(42);
        let replacement_same_hour = test_world_frame(42);
        let expected = ObserverOccupationOutcome {
            surface: None,
            unavailable_reason: Some("cache identity test".into()),
        };
        let cache = Mutex::new(Some(OccupationOutcomeCacheEntry {
            frame: Arc::clone(&frame),
            cache_key_country: game_ui::stored_country_selection::StoredCountrySelection::of("GER"),
            outcome: expected.clone(),
        }));
        assert_eq!(
            cached_occupation_outcome(&cache, &frame, &game_ui::stored_country_selection::StoredCountrySelection::of("GER")),
            Some(expected)
        );
        assert_eq!(
            cached_occupation_outcome(&cache, &replacement_same_hour, &game_ui::stored_country_selection::StoredCountrySelection::of("GER")),
            None
        );
        assert_eq!(cached_occupation_outcome(&cache, &frame, &game_ui::stored_country_selection::StoredCountrySelection::of("FRA")), None);
    }

    #[test]
    fn xui16_render_plan_covers_every_open_panel_and_closed_replacement() {
        use game_ui::occupation_compositor::OccupationPanel;
        let mut viewer = game_ui::production_renderer::ObserverViewerUiState {
            selected_country: game_ui::stored_country_selection::StoredCountrySelection::of("GER"),
            ..Default::default()
        };
        viewer.occupation.selected_country = "GER".into();

        for panel in [
            OccupationPanel::Territories,
            OccupationPanel::GarrisonActivities,
            OccupationPanel::GarrisonEquipment,
        ] {
            viewer.occupation.panel = panel;
            let plan = plan_xui16_panel(&viewer, None, Some("source:42"))
                .expect("open exact panel route")
                .expect("open panel must compose");
            assert_eq!(plan.panel, panel);
            assert_eq!(plan.selected_country, "GER");
            assert_eq!(plan.source_record_id, "source:42");
        }

        viewer.occupation.panel = OccupationPanel::Closed;
        assert_eq!(
            plan_xui16_panel(&viewer, Some("stale failure"), Some("stale source"))
                .expect("closed panel replaces any stale overlay"),
            None
        );
    }

    #[test]
    fn xui16_render_plan_fails_closed_without_exact_surface() {
        use game_ui::occupation_compositor::OccupationPanel;
        let mut viewer = game_ui::production_renderer::ObserverViewerUiState {
            selected_country: game_ui::stored_country_selection::StoredCountrySelection::of("GER"),
            ..Default::default()
        };
        viewer.occupation.selected_country = "GER".into();
        for panel in [
            OccupationPanel::Territories,
            OccupationPanel::GarrisonActivities,
            OccupationPanel::GarrisonEquipment,
        ] {
            viewer.occupation.panel = panel;
            assert!(plan_xui16_panel(&viewer, None, None).is_err());
            assert!(plan_xui16_panel(&viewer, Some("producer gap"), None).is_err());
        }
    }

    #[test]
    fn xui16_product_layer_executor_draws_base_then_each_open_panel_overlay() {
        use game_ui::occupation_compositor::OccupationPanel;
        let mut viewer = game_ui::production_renderer::ObserverViewerUiState {
            selected_country: game_ui::stored_country_selection::StoredCountrySelection::of("GER"),
            ..Default::default()
        };
        viewer.occupation.selected_country = "GER".into();
        for (panel, overlay) in [
            (OccupationPanel::Territories, "territories"),
            (OccupationPanel::GarrisonActivities, "activities"),
            (OccupationPanel::GarrisonEquipment, "equipment"),
        ] {
            viewer.occupation.panel = panel;
            let planned = plan_xui16_panel(&viewer, None, Some("source:42"))
                .expect("exact panel plan")
                .map(|_| overlay);
            let mut draws = Vec::new();
            let output = execute_observer_layers(
                &"hud",
                Ok::<_, &'static str>(planned),
                |image| image,
                |image| {
                    draws.push(*image);
                    Ok(())
                },
            )
            .expect("layer execution");
            assert_eq!(output, Some(overlay));
            assert_eq!(draws, ["hud", overlay]);
        }
    }

    #[test]
    fn xui16_product_layer_executor_replaces_stale_overlay_on_close_and_refusal() {
        let mut draws = Vec::new();
        execute_observer_layers(
            &"hud-1",
            Ok::<_, &'static str>(Some("overlay-1")),
            |image| image,
            |image| {
                draws.push(*image);
                Ok(())
            },
        )
        .expect("initial overlay");
        execute_observer_layers(
            &"hud-2",
            Ok::<_, &'static str>(None::<&str>),
            |image| image,
            |image| {
                draws.push(*image);
                Ok(())
            },
        )
        .expect("Closed is base-only");
        let refusal = execute_observer_layers(
            &"hud-3",
            Err::<Option<&str>, _>("missing exact surface"),
            |image| image,
            |image| {
                draws.push(*image);
                Ok(())
            },
        );
        assert_eq!(refusal, Err("missing exact surface"));
        assert_eq!(draws, ["hud-1", "overlay-1", "hud-2", "hud-3"]);
    }

    #[test]
    fn xui08_shipping_route_is_transactional_presented_and_viewer_owned() {
        let production = production_source();
        for required in [
            "xui08_pending_viewer_checkpoint",
            "compose_xui08_peace_conference_surface",
            "Xui08HistoryAnchor::from",
            "XUI08 viewer checkpoint is absent for rejected frame",
            "toggle_xui08",
            "xui08_pointer_press",
            "xui08_wheel_notches_at",
            "compose_packet_xui05_tooltip",
        ] {
            assert!(
                production.contains(required),
                "missing XUI08 route: {required}"
            );
        }
        let app = crate::source_scan::app_production();
        assert!(app.contains("VK_P if shift"));
        assert!(app.contains("close_xui08"));
        assert!(app.contains("xui08_blocks_lower_pointer_input"));
    }

    #[test]
    fn xui08_country_switch_and_resize_preserve_global_state_and_history_authentication() {
        let production = production_source();
        let execute_start = production
            .find("    pub fn execute(\n        &mut self,\n        gfx: &Gfx,")
            .expect("shipping execute");
        let invalidate_start = production[execute_start..]
            .find("    pub fn invalidate(&mut self)")
            .map(|offset| execute_start + offset)
            .expect("resize invalidation");
        let execute = &production[execute_start..invalidate_start];
        assert!(execute.contains("if timeline_discontinuity {"));
        assert!(execute.contains("self.reset_xui23_for_seek(viewer);"));
        assert!(execute.contains("} else if country_changed {"));
        assert!(execute.contains("self.reset_country_dependent_ui_for_country_change(viewer);"));
        assert!(
            execute.find("self.reset_country_dependent_ui_for_country_change(viewer);")
                < execute.find("begin_xui08_staged_candidate("),
            "country-bound reset must precede the XUI08 transaction checkpoint without mutating it"
        );

        let country_reset_start = production
            .find("    pub fn reset_country_dependent_ui_for_country_change(")
            .expect("country-change reset");
        let seek_reset_start = production[country_reset_start..]
            .find("    pub fn reset_xui23_for_seek(")
            .map(|offset| country_reset_start + offset)
            .expect("timeline-seek reset");
        let country_reset = &production[country_reset_start..seek_reset_start];
        for forbidden in [
            "viewer.peace_conference",
            "xui08_presented",
            "xui08_history_anchor",
            "xui08_pending",
            "xui08_cursor",
        ] {
            assert!(
                !country_reset.contains(forbidden),
                "country switch must preserve global XUI08 field {forbidden}"
            );
        }
        let seek_reset_end = production[seek_reset_start..]
            .find("    pub fn toggle_xui11(")
            .map(|offset| seek_reset_start + offset)
            .expect("seek reset boundary");
        let seek_reset = &production[seek_reset_start..seek_reset_end];
        assert!(seek_reset.contains("viewer.peace_conference = Default::default()"));
        assert!(seek_reset.contains("self.xui08_history_anchor = None"));

        let invalidate_end = production[invalidate_start..]
            .find("    pub fn designer_effective_membership(")
            .map(|offset| invalidate_start + offset)
            .expect("resize invalidation boundary");
        let invalidate = &production[invalidate_start..invalidate_end];
        assert!(
            !invalidate.contains("self.xui08_history_anchor = None"),
            "resize invalidates pixels, not authenticated PCS3 continuity"
        );
    }

    mod xui08_availability_tests {
        use super::*;
        use game_ui::peace_conference_ui::{
            PeaceConferenceHistoryTab, PeaceConferenceScrollRegion, PeaceConferenceScrollbarDrag,
            PeaceConferenceViewerState,
        };
        use game_ui::readonly_design_peace::{
            admit_peace_conference, ActivePeaceConferenceSnapshot, DisplayAtom,
            PeaceConferenceReference, PeaceConferenceSnapshot, PeaceParticipantRole,
            PeaceParticipantRow,
        };
        fn xui08_atom(id: &str, value: &str) -> DisplayAtom {
            DisplayAtom {
                source_record_id: id.into(),
                raw_value: value.into(),
                rendered_text: value.into(),
            }
        }

        fn xui08_conference(
            reference_id: u32,
            snapshot_id: &str,
            participant: &str,
        ) -> PeaceConferenceSnapshot {
            let reference = PeaceConferenceReference {
                object_type: 9,
                object_id: reference_id,
            };
            PeaceConferenceSnapshot::Active(ActivePeaceConferenceSnapshot {
                source_record_id: snapshot_id.into(),
                conference_reference: reference,
                conference_id: reference.portable_id(),
                phase: "bid".into(),
                turn: 1,
                remaining_score_turns: 1,
                current_participant_id: participant.into(),
                current_score: xui08_atom("score", "10"),
                score_this_turn: xui08_atom("turn-score", "3"),
                available_state_ids: Vec::new(),
                contested_state_ids: Vec::new(),
                participants: vec![PeaceParticipantRow {
                    source_record_id: "participant-row".into(),
                    ordinal: 0,
                    participant_id: participant.into(),
                    country_tag: "GER".into(),
                    role: PeaceParticipantRole::Victor,
                    score: xui08_atom("participant-score", "10"),
                    score_ratio: xui08_atom("participant-ratio", "100%"),
                    allocation_ids: Vec::new(),
                    result_ids: Vec::new(),
                    bid_count: xui08_atom("participant-bid-count", "0"),
                    contested_bid_count: xui08_atom("participant-contested-count", "0"),
                }],
                allocations: Vec::new(),
                allocation_groups: Vec::new(),
                contested_popups: Vec::new(),
                results: Vec::new(),
                complete: true,
            })
        }

        fn frame(record: &str) -> Xui08PresentedFrame {
            Xui08PresentedFrame {
                frame_hour: 40,
                capture_sequence: 1,
                record_sha256: record.into(),
                admitted: admit_peace_conference(&xui08_conference(40, record, "participant:GER"))
                    .unwrap(),
                input_region: None,
                hit_targets: Vec::new(),
                tooltip_targets: Vec::new(),
            }
        }

        #[test]
        fn presented_gap_revokes_authority_and_recovery_reconciles_retained_intent() {
            let mut presented = Some(frame("first"));
            presented.as_mut().unwrap().hit_targets.push(
                game_ui::peace_conference_ui::PeaceConferenceHitTarget {
                    rect: [0, 0, 20, 20],
                    action: game_ui::peace_conference_ui::PeaceConferenceHitAction::ClosePanel,
                },
            );
            assert!(presented.as_ref().unwrap().hit_targets[0].contains((10, 10)));
            let mut viewer = PeaceConferenceViewerState::default();
            reconcile_xui08_candidate(&mut viewer, presented.as_ref());
            viewer.set_open(true);
            viewer.selected_participant_id = Some("participant:GER".into());
            viewer
                .expanded_participant_ids
                .insert("participant:GER".into());
            viewer.expanded_action_kinds.insert("take_state".into());
            viewer
                .set_scroll(PeaceConferenceScrollRegion::CurrentAndHistory, 12, 30)
                .unwrap();
            let mut input = None;
            stage_xui08_viewer_input(&mut input, &viewer).history_tab =
                PeaceConferenceHistoryTab::Beneficiaries;
            let mut checkpoint = None;
            begin_xui08_staged_candidate(&mut viewer, &mut input, &mut checkpoint).unwrap();
            let mut candidate = None;
            let mut staged = true;
            reconcile_xui08_candidate(&mut viewer, candidate.as_ref());
            assert!(
                presented.is_some(),
                "staging does not replace the presented frame"
            );
            assert!(viewer.snapshot_identity().is_none());
            presented = commit_xui08_staged_candidate(&mut candidate, &mut staged, &mut checkpoint)
                .unwrap();
            assert!(
                presented.is_none(),
                "no old render, hit or tooltip targets survive Present"
            );
            assert!(!xui08_blocks_lower_pointer_input_state(
                &viewer, None, 10, 10
            ));
            assert!(viewer.open);
            assert_eq!(viewer.history_tab, PeaceConferenceHistoryTab::Beneficiaries);
            assert_eq!(viewer.current_and_history_scroll_px, 12);
            assert_eq!(
                viewer.selected_participant_id.as_deref(),
                Some("participant:GER")
            );
            assert!(viewer.expanded_participant_ids.contains("participant:GER"));
            assert!(viewer.expanded_action_kinds.contains("take_state"));

            begin_xui08_staged_candidate(&mut viewer, &mut input, &mut checkpoint).unwrap();
            candidate = Some(frame("recovery"));
            staged = true;
            reconcile_xui08_candidate(&mut viewer, candidate.as_ref());
            presented = commit_xui08_staged_candidate(&mut candidate, &mut staged, &mut checkpoint)
                .unwrap();
            assert_eq!(presented.as_ref().unwrap().record_sha256, "recovery");
            assert_eq!(viewer.snapshot_identity(), Some("recovery"));
            assert_eq!(
                viewer.history_tab,
                PeaceConferenceHistoryTab::Victors,
                "current authenticated roster has no beneficiaries"
            );
            assert_eq!(viewer.current_and_history_scroll_px, 12);
        }

        #[test]
        fn rejected_gap_and_close_restore_the_presented_viewer_transaction() {
            for disposition in ["skipped", "occluded", "execute-error"] {
                let presented = Some(frame("first"));
                let mut viewer = PeaceConferenceViewerState::default();
                reconcile_xui08_candidate(&mut viewer, presented.as_ref());
                viewer.set_open(true);
                viewer.active_scrollbar_drag = Some(PeaceConferenceScrollbarDrag {
                    region: PeaceConferenceScrollRegion::CurrentAndHistory,
                    grab_offset_y: 3,
                });
                let before = viewer.clone();
                let mut input = None;
                stage_xui08_closed(&mut input, &viewer);
                let mut checkpoint = None;
                begin_xui08_staged_candidate(&mut viewer, &mut input, &mut checkpoint).unwrap();
                let mut candidate = None;
                let mut staged = true;
                reconcile_xui08_candidate(&mut viewer, candidate.as_ref());
                assert!(!viewer.open);
                assert!(viewer.active_scrollbar_drag.is_none());
                rollback_xui08_staged_candidate(
                    &mut viewer,
                    &mut checkpoint,
                    &mut candidate,
                    &mut staged,
                )
                .unwrap();
                assert_eq!(viewer, before, "{disposition}");
                assert_eq!(presented.as_ref().unwrap().record_sha256, "first");
                assert!(candidate.is_none());
                assert!(!staged);
            }
        }
    }

    #[test]
    fn xui08_real_input_sequence_commits_only_present_and_rolls_back_reject_or_error() {
        use game_ui::peace_conference_ui::{PeaceConferenceHistoryTab, PeaceConferenceViewerState};

        let mut viewer = PeaceConferenceViewerState::default();
        viewer.set_open(true);
        viewer.history_tab = PeaceConferenceHistoryTab::Victors;
        let mut pending_input = None;
        let mut checkpoint = None;

        stage_xui08_viewer_input(&mut pending_input, &viewer).history_tab =
            PeaceConferenceHistoryTab::Beneficiaries;
        assert_eq!(viewer.history_tab, PeaceConferenceHistoryTab::Victors);
        begin_xui08_staged_candidate(&mut viewer, &mut pending_input, &mut checkpoint).unwrap();
        assert_eq!(viewer.history_tab, PeaceConferenceHistoryTab::Beneficiaries);

        let mut pending = Some(1_u8);
        let mut staged = true;
        rollback_xui08_staged_candidate(&mut viewer, &mut checkpoint, &mut pending, &mut staged)
            .unwrap();
        assert_eq!(viewer.history_tab, PeaceConferenceHistoryTab::Victors);
        assert!(pending.is_none());
        assert!(!staged);

        stage_xui08_viewer_input(&mut pending_input, &viewer).set_open(false);
        assert!(viewer.open);
        begin_xui08_staged_candidate(&mut viewer, &mut pending_input, &mut checkpoint).unwrap();
        pending = Some(2);
        staged = true;
        rollback_xui08_staged_candidate(&mut viewer, &mut checkpoint, &mut pending, &mut staged)
            .unwrap();
        assert!(viewer.open);

        stage_xui08_viewer_input(&mut pending_input, &viewer).set_open(false);
        begin_xui08_staged_candidate(&mut viewer, &mut pending_input, &mut checkpoint).unwrap();
        pending = Some(3);
        staged = true;
        assert_eq!(
            commit_xui08_staged_candidate(&mut pending, &mut staged, &mut checkpoint).unwrap(),
            Some(3)
        );
        assert!(!viewer.open);
        assert!(checkpoint.is_none());
    }

    #[test]
    fn xui08_escape_closes_popup_before_presented_panel() {
        use game_ui::peace_conference_ui::PeaceConferenceViewerState;
        let mut state = PeaceConferenceViewerState::default();
        state.set_open(true);
        state.select_allocation("allocation:1".into());
        assert!(close_xui08_topmost_state(&mut state));
        assert!(state.open);
        assert!(state.selected_allocation_id.is_none());
        assert!(close_xui08_topmost_state(&mut state));
        assert!(!state.open);
        assert!(!close_xui08_topmost_state(&mut state));
    }

    #[test]
    fn xui08_present_reject_and_execute_error_transactions_are_behavioral() {
        use game_ui::peace_conference_ui::PeaceConferenceViewerState;

        let mut present_pending = Some("candidate");
        let mut present_staged = true;
        let mut present_checkpoint = Some(PeaceConferenceViewerState::default());
        assert_eq!(
            commit_xui08_staged_candidate(
                &mut present_pending,
                &mut present_staged,
                &mut present_checkpoint,
            )
            .expect("Present commits the staged candidate"),
            Some("candidate")
        );
        assert!(!present_staged);
        assert!(present_pending.is_none());
        assert!(present_checkpoint.is_none());

        for disposition in ["Reject", "execute error"] {
            let mut checkpoint = PeaceConferenceViewerState::default();
            checkpoint.set_open(true);
            checkpoint.selected_participant_id = Some("participant:3:47".into());
            let mut viewer = PeaceConferenceViewerState::default();
            viewer.selected_participant_id = Some("staged-mutation".into());
            let mut pending = Some("candidate");
            let mut staged = true;
            let mut saved = Some(checkpoint.clone());
            rollback_xui08_staged_candidate(&mut viewer, &mut saved, &mut pending, &mut staged)
                .unwrap_or_else(|error| panic!("{disposition} rollback failed: {error}"));
            assert_eq!(viewer, checkpoint, "{disposition} did not restore viewer");
            assert!(saved.is_none(), "{disposition} retained checkpoint");
            assert!(pending.is_none(), "{disposition} retained candidate");
            assert!(!staged, "{disposition} retained staged flag");
        }

        let mut viewer = PeaceConferenceViewerState::default();
        let mut no_checkpoint = None;
        let mut pending = Some("candidate");
        let mut staged = true;
        assert!(rollback_xui08_staged_candidate(
            &mut viewer,
            &mut no_checkpoint,
            &mut pending,
            &mut staged,
        )
        .is_err());
        assert_eq!(pending, Some("candidate"));
        assert!(staged);
    }

    #[test]
    fn xui09_shipping_route_is_presented_transactional_and_not_selected_present() {
        let runtime = crate::source_scan::ui_runtime_production();
        let app = crate::source_scan::app_production();
        for required in [
            "observer_research_from_complete_frame(",
            "self.xui09_pending_staged = true",
            "self.xui09_presented = self.xui09_pending.take()",
            "XUI09 viewer checkpoint is absent for rejected frame",
            "self.clear_execute_candidates()",
            "compose_xui09_research_surface(",
            "xui09_tooltip_payloads(",
            "pub fn toggle_xui09(",
            "pub fn cycle_xui09_folder(",
            "pub fn xui09_pointer_press(",
            "pub fn xui09_pointer_release(",
            "close_xui09_after_composition_failure(",
        ] {
            assert!(
                runtime.contains(required),
                "missing XUI09 runtime contract: {required}"
            );
        }
        let product_start = runtime
            .find("pub struct ProductObserverUiRuntime")
            .expect("product runtime");
        let legacy_start = runtime[product_start..]
            .find("struct ProductSelectedPresentPerformanceRuntime")
            .map(|offset| product_start + offset)
            .expect("post-observer runtime boundary");
        let product = &runtime[product_start..legacy_start];
        assert!(!product.contains("admit_xui09_selected_present_route"));
        assert!(!product.contains("XUI09_G4_COMPLETED:complete selected-country"));
        for required in [
            "VK_E if shift && pov.selection().is_selected()",
            "cycle_xui09_folder",
            "xui09_blocks_lower_pointer_input",
            "xui09_pointer_move",
            "xui09_pointer_press",
            "xui09_pointer_release",
            "reset_country_dependent_ui_for_country_change",
        ] {
            assert!(
                app.contains(required),
                "missing XUI09 shipping input route: {required}"
            );
        }
        assert_eq!(
            app.matches("apply_selected_country_identity_with_runtime_reset(")
                .count(),
            4,
            "the common country reset must run for Tab, digit selection, and frame reconciliation",
        );
    }

    fn xui09_test_presented_frame(
        country: &str,
        input_region: Option<game_ui::research_compositor::ResearchInputRegion>,
    ) -> Xui09PresentedFrame {
        Xui09PresentedFrame {
            country_tag: country.into(),
            frame_hour: 25,
            available_folders: vec!["industry_folder".into()],
            input_region,
            node_hit_targets: Vec::new(),
            model: game_ui::research_observer::ObserverResearchModel {
                authority: game_ui::research_observer::XUI09_OBSERVER_AUTHORITY,
                selected_country: country.into(),
                world_hour: 25,
                research_slot_count: 0,
                nodes: Vec::new(),
                refused_countries: Vec::new(),
                localisation_misses: Default::default(),
            },
        }
    }

    #[test]
    fn xui09_initial_open_without_presented_geometry_absorbs_lower_pointer_input() {
        let presented = xui09_test_presented_frame("GER", None);
        assert!(
            xui09_matching_presented_surface(true, &game_ui::stored_country_selection::StoredCountrySelection::of("GER"), Some(&presented),).is_none(),
            "an uncomposed frame must not authorize selection"
        );
        assert!(xui09_blocks_lower_pointer_input_for_state(
            true,
            &game_ui::stored_country_selection::StoredCountrySelection::of("GER"),
            Some(&presented),
            i32::MAX,
            i32::MIN,
        ));
        assert!(xui09_blocks_lower_pointer_input_for_state(
            true,
            &game_ui::stored_country_selection::StoredCountrySelection::of("ENG"),
            Some(&presented),
            0,
            0,
        ));
        assert!(!xui09_blocks_lower_pointer_input_for_state(
            false,
            &game_ui::stored_country_selection::StoredCountrySelection::of("GER"),
            Some(&presented),
            0,
            0,
        ));
    }

    #[test]
    fn xui09_rejected_composition_closes_panel_and_releases_pointer_state() {
        let mut viewer = game_ui::research_compositor::ResearchViewerState {
            panel_open: true,
            selected_folder: Some("industry_folder".into()),
            pan_pixels: [12, -4],
            selected_technology_id: Some("basic_machine_tools".into()),
        };
        let mut drag_cursor = Some([40, 50]);
        let mut cursor = Some((40, 50));
        close_xui09_after_composition_failure(&mut viewer, &mut drag_cursor, &mut cursor);
        assert!(!viewer.panel_open);
        assert!(viewer.selected_technology_id.is_none());
        assert!(drag_cursor.is_none());
        assert!(cursor.is_none());
        assert!(!xui09_blocks_lower_pointer_input_for_state(
            viewer.panel_open,
            &game_ui::stored_country_selection::StoredCountrySelection::of("GER"),
            None,
            40,
            50,
        ));
    }

    #[test]
    fn xui09_country_change_clears_active_capture_and_presented_identity() {
        let mut viewer = game_ui::research_compositor::ResearchViewerState {
            panel_open: true,
            selected_folder: Some("industry_folder".into()),
            pan_pixels: [12, -4],
            selected_technology_id: Some("basic_machine_tools".into()),
        };
        let mut presented = Some(xui09_test_presented_frame(
            "GER",
            Some(game_ui::research_compositor::ResearchInputRegion {
                x: 10,
                y: 20,
                width: 300,
                height: 200,
            }),
        ));
        let mut pending = Some(xui09_test_presented_frame("ENG", None));
        let mut pending_staged = true;
        let mut checkpoint = Some(viewer.clone());
        let mut drag_cursor = Some([40, 50]);
        let mut cursor = Some((40, 50));
        reset_xui09_country_bound_state(
            &mut viewer,
            &mut presented,
            &mut pending,
            &mut pending_staged,
            &mut checkpoint,
            &mut drag_cursor,
            &mut cursor,
        );
        assert_eq!(
            viewer,
            game_ui::research_compositor::ResearchViewerState::default()
        );
        assert!(presented.is_none());
        assert!(pending.is_none());
        assert!(!pending_staged);
        assert!(checkpoint.is_none());
        assert!(drag_cursor.is_none());
        assert!(cursor.is_none());
    }

    #[test]
    fn xui09_selected_main_or_subtechnology_drives_persistent_inspection() {
        let targets = vec![
            game_ui::research_compositor::ResearchNodeHitTarget {
                technology_id: "parent".into(),
                x: 0,
                y: 0,
                width: 20,
                height: 20,
                child_path: vec![1],
            },
            game_ui::research_compositor::ResearchNodeHitTarget {
                technology_id: "variant".into(),
                x: 30,
                y: 0,
                width: 10,
                height: 10,
                child_path: vec![1, 4],
            },
        ];
        let selected = xui09_inspection_target(&targets, Some("variant"), Some((5, 5)))
            .expect("selected subtechnology remains the inspection owner");
        assert_eq!(selected.technology_id, "variant");
        assert_eq!(selected.child_path, vec![1, 4]);

        let hovered = xui09_inspection_target(&targets, None, Some((5, 5)))
            .expect("unselected hover inspection");
        assert_eq!(hovered.technology_id, "parent");
    }

    #[test]
    fn observer_frame_rejection_restores_all_country_and_feature_viewer_state() {
        use game_ui::readonly_design_peace::DesignerViewKind;

        let designer = game_ui::design_catalog::ObserverDesignerViewState {
            family: DesignerViewKind::Equipment,
            anchor_design_id: "7:70".into(),
            anchor_catalogue_hour: 20,
            anchor_catalogue_owner_tag: game_ui::stored_country_selection::StoredCountrySelection::of("GER"),
            anchor_catalogue_family: DesignerViewKind::Equipment,
            selected_design_id: Some("7:70".into()),
            our_filter_enabled: true,
            foreign_filter_enabled: false,
            obsolete_filter_enabled: false,
            excluded_country_filter_tags: vec!["ENG".into()],
            scroll_wheel_steps: 3,
            country_scroll_wheel_steps: 2,
        };
        for disposition in ["skipped", "occluded", "execute-error"] {
            let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();
            viewer.selected_country = game_ui::stored_country_selection::StoredCountrySelection::of("GER");
            viewer.theatre_settings_land_id = Some(41);
            viewer.designer = Some(designer.clone());
            viewer.focus.open = true;
            viewer.focus.pan_pixels = [12, -8];
            viewer.focus.selected_focus_id = Some("focus_original".into());
            viewer.production_lines.panel_open = true;
            viewer.production_lines.scroll_offset_px = 18;
            viewer.trade_panel_open = true;
            viewer.trade.selected_resource_index = 3;
            viewer.occupation.selected_country = "GER".into();
            viewer.occupation.territory_scroll_row = 4;
            viewer.character_selection.unit_leader_ids = vec![73];
            viewer.character_selection.scroll_row = 2;
            viewer.abilities.open = true;
            viewer.abilities.animation_elapsed_micros = 5_000;
            let transaction = ObserverViewerPresentTransaction::begin(&viewer);

            viewer.selected_country = game_ui::stored_country_selection::StoredCountrySelection::of("ENG");
            viewer.theatre_settings_land_id = Some(99);
            viewer.designer = None;
            viewer.focus.pan_pixels = [90, 91];
            viewer.focus.selected_focus_id = Some("focus_candidate".into());
            viewer.production_lines.panel_open = false;
            viewer.production_lines.scroll_offset_px = 99;
            viewer.trade_panel_open = false;
            viewer.trade.selected_resource_index = 6;
            viewer.occupation.selected_country = "ENG".into();
            viewer.occupation.territory_scroll_row = 11;
            viewer.character_selection.unit_leader_ids.clear();
            viewer.character_selection.scroll_row = 9;
            viewer.abilities.open = false;
            viewer.abilities.animation_elapsed_micros = 91_000;
            transaction.reject(&mut viewer);

            assert_eq!(
                viewer.selected_country.is("GER"),
                true,
                "{disposition}"
            );
            assert_eq!(viewer.theatre_settings_land_id, Some(41), "{disposition}");
            assert_eq!(viewer.designer, Some(designer.clone()), "{disposition}");
            assert_eq!(viewer.focus.pan_pixels, [12, -8], "{disposition}");
            assert_eq!(
                viewer.focus.selected_focus_id.as_deref(),
                Some("focus_original"),
                "{disposition}"
            );
            assert!(viewer.production_lines.panel_open, "{disposition}");
            assert_eq!(
                viewer.production_lines.scroll_offset_px, 18,
                "{disposition}"
            );
            assert!(viewer.trade_panel_open, "{disposition}");
            assert_eq!(viewer.trade.selected_resource_index, 3, "{disposition}");
            assert_eq!(viewer.occupation.selected_country, "GER", "{disposition}");
            assert_eq!(viewer.occupation.territory_scroll_row, 4, "{disposition}");
            assert_eq!(
                viewer.character_selection.unit_leader_ids,
                vec![73],
                "{disposition}"
            );
            assert_eq!(viewer.character_selection.scroll_row, 2, "{disposition}");
            assert!(viewer.abilities.open, "{disposition}");
            assert_eq!(
                viewer.abilities.animation_elapsed_micros, 5_000,
                "{disposition}"
            );
        }
    }

    #[test]
    fn resize_invalidation_survives_rejection_without_committing_other_viewer_mutations() {
        for disposition in ["skipped", "occluded", "execute-error"] {
            let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();
            viewer.selected_country = game_ui::stored_country_selection::StoredCountrySelection::of("GER");
            viewer.trade_panel_open = true;
            viewer.trade.selected_resource_index = 3;
            viewer.politics.panel_open = true;
            viewer.politics.country_tag = game_ui::stored_country_selection::StoredCountrySelection::of("GER");
            viewer.politics.displayed_scroll_fixed = 12_000;
            viewer.politics.target_scroll_fixed = 18_000;
            viewer.politics.selected_idea_id = Some("original_idea".into());

            let mut transaction = ObserverViewerPresentTransaction::begin(&viewer);

            viewer.selected_country = game_ui::stored_country_selection::StoredCountrySelection::of("ENG");
            viewer.trade_panel_open = false;
            viewer.trade.selected_resource_index = 7;

            // XUI08/XUI15 pointer releases and XUI11 state tied to the retired
            viewer.politics.reset_frame_bound_state();
            transaction.make_politics_frame_invalidation_durable(&viewer);
            viewer.politics.displayed_scroll_fixed = 99_000;
            viewer.politics.target_scroll_fixed = 101_000;
            viewer.politics.selected_idea_id = Some("candidate_idea".into());

            transaction.reject(&mut viewer);

            assert_eq!(
                viewer.selected_country.is("GER"),
                true,
                "{disposition}"
            );
            assert!(viewer.trade_panel_open, "{disposition}");
            assert_eq!(viewer.trade.selected_resource_index, 3, "{disposition}");
            assert!(viewer.politics.panel_open, "{disposition}");
            assert!(viewer.politics.country_tag.is("GER"), "{disposition}");
            assert_eq!(viewer.politics.displayed_scroll_fixed, 0, "{disposition}");
            assert_eq!(viewer.politics.target_scroll_fixed, 0, "{disposition}");
            assert!(viewer.politics.selected_idea_id.is_none(), "{disposition}");
        }
    }

    #[test]
    fn observer_frame_present_commits_event_then_execute_candidate_state() {
        let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();
        viewer.focus.open = true;
        viewer.focus.selected_focus_id = Some("event-selected".into());
        let mut backing: Option<Xui10PresentedFrame> = None;
        let transaction = ObserverViewerPresentTransaction::begin(&viewer);

        viewer.focus.selected_focus_id = Some("event-mutated".into());
        viewer.theatre_settings_land_id = Some(77);
        viewer.focus.selected_focus_id = Some("execute-reconciled".into());
        backing = Some(Xui10PresentedFrame {
            country_tag: "GER".into(),
            frame_hour: 77,
            input_region: game_ui::focus_observer::ObserverFocusInputRegion {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            node_regions: Vec::new(),
            progress: None,
        });
        transaction.commit();

        assert_eq!(viewer.theatre_settings_land_id, Some(77));
        assert_eq!(
            viewer.focus.selected_focus_id.as_deref(),
            Some("execute-reconciled")
        );
        assert_eq!(backing.as_ref().map(|frame| frame.frame_hour), Some(77));
    }

    #[test]
    fn production_present_owner_commits_xui12_xui15_only_for_presented() {
        let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();
        viewer.production_lines.panel_open = true;
        viewer.production_lines.scroll_offset_px = 7;
        viewer.trade_panel_open = true;
        viewer.trade.selected_resource_index = 2;
        viewer.trade.catalogue_scroll.target_fixed = 12_000;

        let mut owner = ObserverUiPresentTransactionOwner::default();
        owner.xui15_presented = Some(Xui15InteractionSurface::fixture([800, 600]));
        owner.begin(&viewer).expect("production owner begins");
        viewer.production_lines.scroll_offset_px = 99;
        viewer.trade.selected_resource_index = 5;
        viewer.trade.catalogue_scroll.target_fixed = 48_000;
        owner.stage_xui15(Xui15InteractionSurface::fixture([1920, 1080]));
        owner
            .finish_disposition(&mut viewer, ObserverUiFrameDisposition::Presented, Some(41))
            .expect("Presented candidate commits through the production owner");
        assert_eq!(viewer.production_lines.scroll_offset_px, 41);
        assert_eq!(viewer.trade.selected_resource_index, 5);
        assert_eq!(viewer.trade.catalogue_scroll.target_fixed, 48_000);
        assert_eq!(
            owner.presented_xui15().map(|surface| surface.viewport),
            Some([1920, 1080])
        );
        assert!(owner.viewer.is_none());
        assert!(owner.xui15_checkpoint.is_none());
        assert!(owner.xui15_pending.is_none());

        for (disposition, rejected_viewport) in [
            (ObserverUiFrameDisposition::Skipped, [640, 480]),
            (ObserverUiFrameDisposition::Occluded, [1024, 768]),
            (ObserverUiFrameDisposition::ExecuteError, [1280, 720]),
        ] {
            owner.begin(&viewer).expect("production owner begins");
            viewer.production_lines.scroll_offset_px = 90;
            viewer.trade_panel_open = false;
            viewer.trade.selected_resource_index = 8;
            viewer.trade.catalogue_scroll.target_fixed = 96_000;
            owner.stage_xui15(Xui15InteractionSurface::fixture(rejected_viewport));
            owner
                .finish_disposition(&mut viewer, disposition, Some(73))
                .expect("undisplayed candidate rejects through the production owner");
            assert_eq!(
                viewer.production_lines.scroll_offset_px, 41,
                "{disposition:?}"
            );
            assert!(viewer.trade_panel_open, "{disposition:?}");
            assert_eq!(viewer.trade.selected_resource_index, 5, "{disposition:?}");
            assert_eq!(
                viewer.trade.catalogue_scroll.target_fixed, 48_000,
                "{disposition:?}"
            );
            assert_eq!(
                owner.presented_xui15().map(|surface| surface.viewport),
                Some([1920, 1080]),
                "{disposition:?}"
            );
            assert!(owner.viewer.is_none(), "{disposition:?}");
            assert!(owner.xui15_checkpoint.is_none(), "{disposition:?}");
            assert!(owner.xui15_pending.is_none(), "{disposition:?}");
        }
    }

    #[test]
    fn resize_present_then_mouse_move_cannot_resume_xui08_or_xui15_drag() {
        use game_ui::peace_conference_ui::{
            PeaceConferenceScrollRegion, PeaceConferenceScrollbarDrag,
        };
        use game_ui::trade_layout::{TradeScrollTarget, TradeScrollbarDrag};

        let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();
        viewer.trade.active_scrollbar_drag = Some(TradeScrollbarDrag {
            target: TradeScrollTarget::Catalogue,
            grab_offset_y: 5,
        });
        viewer.peace_conference.active_scrollbar_drag = Some(PeaceConferenceScrollbarDrag {
            region: PeaceConferenceScrollRegion::AvailableActions,
            grab_offset_y: 7,
        });

        let mut owner = ObserverUiPresentTransactionOwner::default();
        owner.xui15_presented = Some(Xui15InteractionSurface::fixture([800, 600]));
        owner.begin(&viewer).expect("begin pre-resize frame");

        owner.invalidate_runtime_surfaces();
        viewer.trade.active_scrollbar_drag = None;
        viewer.peace_conference.release_active_scrollbar();
        owner
            .make_resize_invalidation_durable(&viewer)
            .expect("make resize pointer release durable");
        owner.stage_xui15(Xui15InteractionSurface::fixture([1920, 1080]));
        owner
            .finish_disposition(&mut viewer, ObserverUiFrameDisposition::Presented, None)
            .expect("Present resized candidate");

        assert!(viewer.trade.active_scrollbar_drag.is_none());
        assert!(viewer.peace_conference.active_scrollbar_drag.is_none());
        assert_eq!(
            owner.presented_xui15().map(|surface| surface.viewport),
            Some([1920, 1080])
        );
    }

    #[test]
    fn execute_error_rollback_clears_early_and_late_candidates_before_retry() {
        for injected_failure in [
            ObserverUiExecuteStage::EarlyCandidates,
            ObserverUiExecuteStage::LateCandidates,
        ] {
            let mut lifecycle = ObserverUiExecuteLifecycle::default();
            lifecycle.begin().expect("first frame starts");
            if injected_failure == ObserverUiExecuteStage::LateCandidates {
                lifecycle.mark_authored_candidate();
                lifecycle.mark_late_candidates();
            }
            assert_eq!(lifecycle.stage, injected_failure);
            assert!(
                lifecycle.begin().is_err(),
                "a failed candidate blocks reuse until rollback"
            );
            assert!(
                lifecycle
                    .finish_ownerless_execute_error(false, false)
                    .expect("ownerless execute-error rollback"),
                "the real ownerless rollback branch consumed {injected_failure:?}"
            );
            lifecycle
                .begin()
                .expect("successful retry starts after injected failure rollback");
            assert_eq!(
                lifecycle.stage,
                ObserverUiExecuteStage::EarlyCandidates,
                "retry starts from a clean candidate lifecycle"
            );
        }

        let source = production_source();
        let clear_start = source
            .find("    fn clear_execute_candidates(&mut self)")
            .expect("candidate cleanup helper");
        let rollback_start = source[clear_start..]
            .find("    pub fn rollback_after_execute_error(")
            .map(|offset| clear_start + offset)
            .expect("execute rollback");
        let clear = &source[clear_start..rollback_start];
        for slot in [
            "self.present_transaction.xui15_pending = None",
            "self.xui05_pending_owner = None",
            "self.xui05_pending_controller_checkpoint = None",
            "self.xui23_pending = None",
            "self.xui10_pending = None",
            "self.xui09_pending = None",
            "self.xui09_pending_staged = false",
            "self.xui09_pending_viewer_checkpoint = None",
            "self.xui11_pending = None",
            "self.xui11_pending_staged = false",
            "self.xui11_pending_viewer_checkpoint = None",
            "self.xui18_pending = None",
            "self.xui18_pending_staged = false",
            "self.xui18_pending_viewer_checkpoint = None",
            "self.xui19_pending = None",
            "self.xui19_pending_staged = false",
            "self.xui19_pending_viewer_checkpoint = None",
            "self.xui08_pending = None",
            "self.xui08_pending_staged = false",
            "self.xui08_pending_viewer_checkpoint = None",
            "self.xui08_pending_viewer_input = None",
            "self.xui14_pending = None",
            "self.xui14_pending_staged = false",
            "self.xui14_pending_viewer_checkpoint = None",
            "self.xui20_pending = None",
            "self.xui20_pending_staged = false",
            "self.xui20_pending_viewer_checkpoint = None",
            "self.xui20_pending_cursor_checkpoint = None",
            "self.xui21_pending = None",
            "self.xui21_pending_staged = false",
            "self.xui21_pending_viewer_checkpoint = None",
        ] {
            assert!(
                clear.contains(slot),
                "rollback omitted candidate slot {slot}"
            );
        }
        let rollback_end = source[rollback_start..]
            .find("    pub fn reject_unpresented_authored_frame(")
            .map(|offset| rollback_start + offset)
            .expect("rollback boundary");
        let rollback = &source[rollback_start..rollback_end];
        let frame_precondition = source
            .find("if self.observer_frame_checkpoint.is_none()")
            .expect("execute frame-owner precondition");
        let lifecycle_begin = source
            .find("self.execute_lifecycle.begin()?")
            .expect("execute lifecycle begin");
        assert!(
            frame_precondition < lifecycle_begin,
            "an ownerless execute error must not dirty the lifecycle"
        );
        let ownerless_finish = rollback
            .find(".finish_ownerless_execute_error(frame_pending, owner_pending)?")
            .expect("real ownerless execute-error rollback branch");
        let reject_authored = rollback
            .find(".reject_unpresented_frame()")
            .expect("late authored candidate rollback");
        let clear_ownerless_candidates = rollback
            .find("self.clear_execute_candidates()")
            .expect("ownerless candidate cleanup");
        let clear_owned_candidates = rollback[reject_authored..]
            .find("self.clear_execute_candidates()")
            .map(|offset| reject_authored + offset)
            .expect("owned early and late candidate cleanup");
        let restore_frame = rollback
            .find("frame_checkpoint.restore(viewer, self)")
            .expect("runtime checkpoint restore");
        let restore_viewer = rollback
            .find("self.present_transaction.reject(viewer)")
            .expect("viewer checkpoint restore");
        assert!(
            ownerless_finish < clear_ownerless_candidates
                && reject_authored < clear_owned_candidates
                && clear_owned_candidates < restore_frame
                && restore_frame < restore_viewer
        );
        assert!(rollback.contains("self.execute_lifecycle.finish()"));
    }

    #[test]
    fn xui14_same_batch_close_removes_xui08_pointer_ownership() {
        let mut presented = game_ui::peace_conference_ui::PeaceConferenceViewerState::default();
        presented.set_open(true);
        let mut pending = None;

        stage_xui08_closed(&mut pending, &presented);
        let current = xui08_current_viewer_state(&pending, &presented);
        assert!(!current.open);
        assert!(!xui08_blocks_lower_pointer_input_state(
            current, None, 50, 50
        ));
        assert!(
            presented.open,
            "last Presented state remains immutable until disposition"
        );
    }

    #[test]
    fn capture_lost_release_survives_xui08_rejected_frame_rollback() {
        use game_ui::peace_conference_ui::{
            PeaceConferenceScrollRegion, PeaceConferenceScrollbarDrag, PeaceConferenceViewerState,
        };

        let mut presented = PeaceConferenceViewerState::default();
        presented.set_open(true);
        presented.active_scrollbar_drag = Some(PeaceConferenceScrollbarDrag {
            region: PeaceConferenceScrollRegion::AvailableActions,
            grab_offset_y: 7,
        });
        let mut pending = None;
        assert!(release_xui08_pointer_capture_durably(
            &mut pending,
            &mut presented
        ));
        assert!(presented.active_scrollbar_drag.is_none());

        // Execute checkpoints the already-released state. Skipped, Occluded,
        let mut checkpoint = None;
        begin_xui08_staged_candidate(&mut presented, &mut pending, &mut checkpoint)
            .expect("begin XUI08 candidate");
        presented.active_scrollbar_drag = Some(PeaceConferenceScrollbarDrag {
            region: PeaceConferenceScrollRegion::CurrentAndHistory,
            grab_offset_y: 11,
        });
        let mut candidate = Some(());
        let mut staged = true;
        rollback_xui08_staged_candidate(
            &mut presented,
            &mut checkpoint,
            &mut candidate,
            &mut staged,
        )
        .expect("reject candidate");
        assert!(presented.active_scrollbar_drag.is_none());
        assert!(candidate.is_none());
        assert!(!staged);
    }

    #[test]
    fn capture_lost_release_survives_xui10_rejected_frame_rollback() {
        let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();
        viewer.focus.open = true;
        viewer.focus.pan_pixels = [12, -4];
        let mut backing: Option<Xui10PresentedFrame> = None;
        let mut drag_cursor = Some([40, 50]);
        let mut checkpoint = (viewer.clone(), backing.clone(), drag_cursor);
        assert!(drag_cursor.take().is_some());
        checkpoint.2 = None;
        viewer.focus.pan_pixels = [90, 91];
        (viewer, backing, drag_cursor) = checkpoint;

        assert_eq!(viewer.focus.pan_pixels, [12, -4]);
        assert!(drag_cursor.is_none(), "OS capture loss survives rejection");
    }

    #[test]
    fn shipping_frame_transaction_begins_before_event_dispatch() {
        let app = crate::source_scan::app_production();
        let begin = app
            .find("begin_observer_frame_transaction(&observer_viewer_ui)")
            .expect("pre-event transaction begin");
        let dispatch = app[begin..]
            .find("for event in events")
            .map(|offset| begin + offset)
            .expect("event dispatch");
        let execute = app[dispatch..]
            .find("product_observer_ui.execute(")
            .map(|offset| dispatch + offset)
            .expect("observer execute");
        assert!(begin < dispatch && dispatch < execute);
        assert!(app.contains("WindowEvent::CaptureLost =>"));
        assert!(app.contains("product_observer_ui.xui08_capture_lost"));
        assert!(app.contains("product_observer_ui.xui10_capture_lost"));
    }
}
