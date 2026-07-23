//! D3D product adapter for the proven game_ui compositor.

use game_ui::assets::AssetResolver;
use game_ui::deployment_queue_state::AdmittedDeploymentQueue;
use game_ui::gfx::SpriteRegistry;
use game_ui::power_balance::PowerBalanceRuntimeSupplement;
use game_ui::power_balance_assets::{
    resolve_power_balance_content_assets, PortablePowerBalanceContentSet,
};
use game_ui::production_renderer::production_ui_variant_key;
use game_ui::variant_admission::{
    DisplayVariantKey, ExactKeyVariantContract, InputVariant, PixelExtent,
};
use replay_core::{ArmyBattleplanOrderKey, RecordedInputVariant, WorldFrame};

use crate::d3d::Gfx;
use crate::text::ImageRenderer;
use crate::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectedPresentUiOutcome {
    Executed {
        operation_count: usize,
        m02_draw_count: usize,
        /// Exact heterogeneous A34 items admitted for the same selected
        /// Present. Their pixels are already owned by the authenticated pass;
        /// this count proves the production render path consumed the typed
        /// model without submitting a second, reconstructed panel.
        decision_panel_item_count: Option<usize>,
        /// Exact A37 typed rows joined by replay_core to this selected Present.
        /// Pixels remain wholly owned by `AuthenticatedUiPass`.
        power_balance_item_count: Option<usize>,
        /// XUI14 panels admitted through the exact A30 selected-Present
        /// companion. `None` is an explicit feature-local omission.
        intelligence_panel_count: Option<usize>,
        /// Feature-local fail-closed reason when XUI14 inputs are absent or
        /// rejected. This never suppresses the authenticated base UI pass.
        intelligence_unavailable_reason: Option<String>,
        /// Typed A30 controller summaries consumed by the selected-POV product
        /// route. `None` remains feature-local unavailable.
        agency_upgrade_count: Option<usize>,
        cipher_target_count: Option<usize>,
        /// X3 licence cards consumed from the typed trade presentation.
        trade_license_count: Option<usize>,
        /// Exact XUI15 canvas drawn from an active, POV-matched MTRA carrier.
        xui15_surface_drawn: bool,
        xui15_unavailable_reason: Option<String>,
        /// Exact XUI16 rows admitted through ProductionUiRenderer for this
        /// selected-Present execution.
        occupation_state_count: Option<usize>,
        occupation_unavailable_reason: Option<String>,
        /// Exact d54 orders for the selected army. `None` means selection or
        /// authenticated carrier admission failed closed.
        battleplan_order_count: Option<usize>,
        /// Whether the optional stable order identity resolved in that same
        /// exact census. This is never inferred from a slot index.
        battleplan_selected_order_present: Option<bool>,
        /// Remaining typed replay models consumed before the authenticated
        /// selected-Present operation frontier executes. These are receipts
        /// for the models behind those pixels, never permission to submit a
        /// second reconstructed panel.
        typed_consumption: SelectedPresentTypedConsumption,
    },
    Omitted(String),
}

/// Pointer-free product-consumption receipt for typed late-RE models whose
/// pixels, when visible, are already owned by `AuthenticatedUiPass`.
/// `None` means feature-local unavailable; measured empty remains `Some(0)`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct SelectedPresentTypedConsumption {
    /// Stable feature-id/reason rows for every local omission represented by
    /// a `None` count below. This prevents a dormant producer from looking
    /// like a measured-empty surface in interactive logs or headless receipts.
    pub feature_local_unavailable: Vec<String>,
    pub national_focus_node_count: Option<usize>,
    pub active_ability_count: Option<usize>,
    pub character_leader_count: Option<usize>,
    pub character_ace_count: Option<usize>,
    pub selected_designer_record_count: Option<usize>,
    pub peace_conference_active: Option<bool>,
    pub orders_group_manager_complete: Option<bool>,
    pub orders_group_selected_army_present: Option<bool>,
    pub combat_detail_admitted_surface_count: Option<usize>,
    pub combat_detail_admitted_row_count: Option<usize>,
    pub fired_event_count: Option<usize>,
    pub fired_event_popup_count: Option<usize>,
    /// X10 direct focus/research/decision/on-action history consumed by the
    /// selected-POV runtime. Evidence-fixture histories remain unavailable.
    pub direct_transition_count: Option<usize>,
    pub trade_resource_pool_count: Option<usize>,
    pub trade_resource_count: Option<usize>,
    pub trade_deal_count: Option<usize>,
    pub trade_market_stockpile_count: Option<usize>,
    pub special_project_breakthrough_count: Option<usize>,
    pub resolved_tooltip_instance_count: Option<usize>,
    /// Exact A6 rows from the bounded CSupplySystem owner census. This is a
    /// product-consumption count only; no province/node identity is inferred
    /// from the legacy supply-ratio surface.
    pub disrupted_supply_exact_row_count: Option<usize>,
    /// Exact foreign-homebase pressure records admitted through the typed D44
    /// view. This is diagnostic consumption only; it never reconstructs rows
    /// from the legacy convoy-pressure wall.
    pub foreign_homebase_exact_row_count: Option<usize>,
    /// Reciprocal operative assignments projected only from complete INGR v4
    /// membership. Legacy topology without operative membership stays local-
    /// unavailable instead of appearing as an empty assignment list.
    pub operative_network_assignment_count: Option<usize>,
    /// Exact d64 lifecycle history. The count is populated only when the
    /// producer's completeness witness is measured true.
    pub delayed_event_lifecycle_count: Option<usize>,
    pub delayed_event_lifecycle_complete: Option<bool>,
    /// X4 ordered hierarchy counts admitted from the selected-POV desktop
    /// presentation model. `Some(0)` is measured empty; any unavailable or
    /// unobtained state leaves all three fields `None` and emits a named reason
    /// in `feature_local_unavailable`. These are model-consumption counts, not
    /// a claim that the deployment window was active or its pixels observed.
    pub deployment_queue_conveyor_count: Option<usize>,
    pub deployment_queue_line_count: Option<usize>,
    pub deployment_queue_equipment_in_transit_count: Option<usize>,
    /// Exact d24 strategic-region weather rows admitted for this frame. This
    /// consumes the typed state; weather particles remain owned by the
    /// authenticated map submit and are never synthesized here.
    pub exact_weather_region_count: Option<usize>,
    /// Exact ACE1 lifecycle rosters, independently of the A39 presentation
    /// subset used by the character panel.
    pub exact_ace_owner_count: Option<usize>,
    pub exact_ace_row_count: Option<usize>,
    /// Exact current-hour B4 monthly sunk-convoy history. D70 aggregates and
    /// route-less terminal evidence never substitute for this field.
    pub exact_sunk_convoy_history_count: Option<usize>,
    /// CAP6/X-COUNTER-BIND is consumed only when the exact selected-Present
    /// frontier contains authenticated `mapicons_container` counter owners.
    /// The HUD commands remain sole pixel authority; these counts prove that
    /// the typed counter model was consumed from the same WorldFrame. Domain
    /// counts stay `None` unless that domain's own authored surface was
    /// authenticated; air remains `None` until an air owner is proven.
    pub counter_hud_authenticated_owner_count: Option<usize>,
    pub counter_hud_authenticated_command_count: Option<usize>,
    pub counter_hud_land_content_count: Option<usize>,
    pub counter_hud_naval_content_count: Option<usize>,
    pub counter_hud_air_content_count: Option<usize>,
    pub counter_exact_supply_row_count: Option<usize>,
    pub counter_exact_organisation_bar_row_count: Option<usize>,
    pub counter_exact_order_kind_row_count: Option<usize>,
    pub counter_evaluated_low_supply_row_count: Option<usize>,
    pub counter_evaluated_dig_in_row_count: Option<usize>,
    pub counter_evaluated_training_row_count: Option<usize>,
    pub counter_evaluated_invalid_orders_row_count: Option<usize>,
    pub counter_evaluated_attrition_row_count: Option<usize>,
    pub counter_exact_active_ability_row_count: Option<usize>,
}

#[derive(Clone, Copy, Default)]
pub struct IntelligenceUiSupplementalInputs<'a> {
    /// Semantic XUI14 rows are supplemental until their full portable producer
    /// is promoted. They can never be used without the replay-carried A30
    /// controller companion and current production selected Present.
    pub trace: Option<&'a game_ui::intelligence_ui::IntelligenceUiTrace>,
}

#[derive(Clone, Copy, Default)]
pub struct ArmyBattleplanUiSupplementalInputs<'a> {
    /// Local product selection joined to the complete d54 manager census.
    pub selected_owner: Option<replay_core::ArmyBattleplanOwnerKey>,
    /// Stable replay identity selected from an already admitted panel row.
    pub selected_order_key: Option<&'a ArmyBattleplanOrderKey>,
}

#[derive(Clone, Copy, Default)]
pub struct OccupationUiSupplementalInputs<'a> {
    /// Complete product-owned XUI16 inputs. Absence is reported as a
    /// feature-local omission and never suppresses the authenticated UI pass.
    pub request: Option<game_ui::occupation_binding::OccupationProductionSurfaceRequest<'a>>,
}

/// Exact selected-Present UI-pass adapter. Unlike the legacy whole-HUD image,
/// this executes the portable operation frontier in place and therefore can
/// interleave M02 at its authenticated global UI ordinal.
pub struct ProductSelectedPresentRuntime {
    pass_identity_sha256: String,
    variant_key: Option<DisplayVariantKey>,
    resolver: AssetResolver,
    registry: SpriteRegistry,
    image: ImageRenderer,
    m02: map_viewer::NativeM02OverlayExecutor,
    power_balance_runtime: Vec<PowerBalanceRuntimeSupplement>,
    power_balance_runtime_error: Option<String>,
    power_balance_assets: Vec<PortablePowerBalanceContentSet>,
    power_balance_asset_error: Option<String>,
    intelligence_panel_count: Option<usize>,
    intelligence_error: Option<String>,
    production_ui_renderer: Option<game_ui::production_renderer::ProductionUiRenderer>,
    occupation_state_count: Option<usize>,
    occupation_error: Option<String>,
    special_project_breakthroughs:
        Vec<game_ui::special_projects::SpecialProjectBreakthroughPresentation>,
    special_project_breakthrough_error: Option<String>,
    selected_pov_runtime: crate::selected_pov_runtime::SelectedPovRuntime,
}

/// Performance-only selected-Present stress adapter.
///
/// This deliberately owns no semantic UI state and is never a replay or RE
/// authority. It reuses the exact production `ImageRenderer` upload/draw path
/// for a contract-pinned number of selected-Present-equivalent GPU operations.
/// The F1 receipt labels the result non-semantic and non-promotional.
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

impl ProductSelectedPresentRuntime {
    /// Build the selected-Present runtime with the exact effective decision
    /// catalog extracted from the authenticated replay package. `None` is an
    /// explicit feature-local omission; installed or developer-tree files are
    /// never consulted as a fallback.
    pub fn new_with_decision_catalog_bytes(
        gfx: &Gfx,
        resolver: &map_viewer::GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        pass: &replay_core::AuthenticatedUiPass,
        decision_catalog_bytes: Option<&[u8]>,
    ) -> Result<Self> {
        let assets =
            game_ui::selected_present::admitted_asset_resolver(resolver, content_admission, pass)?;
        let mut registry = SpriteRegistry::new();
        for file in assets.list_files_recursive("interface", Some("gfx"))? {
            registry.parse_file(&file.path, &file.rel_path)?;
        }
        let m02 = unsafe {
            map_viewer::NativeM02OverlayExecutor::new(gfx.device.as_raw(), gfx.context.as_raw())?
        };
        let selected_pov_runtime =
            crate::selected_pov_runtime::SelectedPovRuntime::new_with_decision_catalog_bytes(
                &assets,
                &registry,
                decision_catalog_bytes,
            );
        let (production_ui_renderer, occupation_error) =
            match game_ui::production_renderer::ProductionUiRenderer::new(
                assets.clone(),
                content_admission,
            ) {
                Ok(renderer) => (Some(renderer), Some(
                    "XUI16 production surface inputs are unavailable".into(),
                )),
                Err(error) => (None, Some(format!(
                    "XUI16 production renderer is feature-local unavailable: {}",
                    error.message
                ))),
            };
        Ok(Self {
            pass_identity_sha256: pass.pass_identity_sha256.clone(),
            variant_key: None,
            resolver: assets,
            registry,
            image: ImageRenderer::new(gfx)?,
            m02,
            power_balance_runtime: Vec::new(),
            power_balance_runtime_error: None,
            power_balance_assets: Vec::new(),
            power_balance_asset_error: None,
            intelligence_panel_count: None,
            intelligence_error: Some("XUI14 supplemental semantic rows are unavailable".into()),
            production_ui_renderer,
            occupation_state_count: None,
            occupation_error,
            special_project_breakthroughs: Vec::new(),
            special_project_breakthrough_error: Some(
                "X9 selected country POV has not been evaluated".into(),
            ),
            selected_pov_runtime,
        })
    }

    pub fn is_for_pass(&self, pass: &replay_core::AuthenticatedUiPass) -> bool {
        self.pass_identity_sha256 == pass.pass_identity_sha256
    }

    /// Last typed A37/A25 join prepared by the production UI runtime for the
    /// selected POV. Failure remains feature-local and cannot suppress the
    /// independently authenticated selected-Present pass.
    pub fn power_balance_runtime(
        &self,
    ) -> std::result::Result<&[PowerBalanceRuntimeSupplement], &str> {
        match self.power_balance_runtime_error.as_deref() {
            Some(error) => Err(error),
            None => Ok(&self.power_balance_runtime),
        }
    }

    /// Root-independent GFX/texture identities for the latest admitted A37
    /// rows. This stays feature-local until the authenticated panel-open pass
    /// exists; a missing BOP dependency cannot contaminate the base HUD pass.
    pub fn power_balance_assets(
        &self,
    ) -> std::result::Result<&[PortablePowerBalanceContentSet], &str> {
        match self.power_balance_asset_error.as_deref() {
            Some(error) => Err(error),
            None => Ok(&self.power_balance_assets),
        }
    }

    pub fn intelligence_panel_count(&self) -> std::result::Result<usize, &str> {
        match self.intelligence_error.as_deref() {
            Some(error) => Err(error),
            None => self.intelligence_panel_count.ok_or("XUI14 admission produced no panel count"),
        }
    }

    /// Exact X9 named breakthrough rows for the selected POV. Raw handles and
    /// unresolved owners never enter this product-facing collection.
    pub fn special_project_breakthroughs(
        &self,
    ) -> std::result::Result<
        &[game_ui::special_projects::SpecialProjectBreakthroughPresentation],
        &str,
    > {
        match self.special_project_breakthrough_error.as_deref() {
            Some(error) => Err(error),
            None => Ok(&self.special_project_breakthroughs),
        }
    }

    pub fn execute(
        &mut self,
        gfx: &Gfx,
        game_resolver: &map_viewer::GameFileResolver,
        prepared_frame: &map_viewer::PreparedProductionD3dFrame,
        world_frame: &WorldFrame,
        pov_tag: Option<&str>,
        runtime_text: Option<&replay_core::RuntimeTextCatalog>,
        army_inputs: ArmyBattleplanUiSupplementalInputs<'_>,
        intelligence_inputs: IntelligenceUiSupplementalInputs<'_>,
        occupation_inputs: OccupationUiSupplementalInputs<'_>,
    ) -> Result<SelectedPresentUiOutcome> {
        self.power_balance_runtime.clear();
        self.power_balance_runtime_error = None;
        self.power_balance_assets.clear();
        self.power_balance_asset_error = None;
        self.intelligence_panel_count = None;
        self.intelligence_error = Some("XUI14 supplemental semantic rows are unavailable".into());
        self.occupation_state_count = None;
        self.occupation_error = Some(if self.production_ui_renderer.is_some() {
            "XUI16 production surface inputs are unavailable".into()
        } else {
            "XUI16 production renderer is feature-local unavailable".into()
        });
        self.special_project_breakthroughs.clear();
        self.special_project_breakthrough_error =
            Some("X9 selected country POV is unavailable".into());
        self.selected_pov_runtime.refresh(
            world_frame,
            pov_tag,
            crate::selected_pov_runtime::SelectedPovSupplementalInputs {
                selected_battleplan_owner: army_inputs.selected_owner,
                selected_order_key: army_inputs.selected_order_key,
                ..Default::default()
            },
            runtime_text,
            &self.registry,
        );
        if let Some(country_tag) = pov_tag {
            match crate::special_projects_runtime::admit_country_special_project_breakthroughs(
                world_frame,
                country_tag,
            ) {
                Ok(rows) => {
                    self.special_project_breakthroughs = rows;
                    self.special_project_breakthrough_error = None;
                }
                Err(error) => self.special_project_breakthrough_error = Some(error),
            }
            match crate::power_balance_runtime::power_balance_runtime_supplements(
                world_frame,
                country_tag,
            ) {
                Ok(runtime) => {
                    for row in &runtime {
                        match resolve_power_balance_content_assets(
                            row,
                            &self.registry,
                            &self.resolver,
                        ) {
                            Ok(assets) => self.power_balance_assets.push(assets),
                            Err(error) => {
                                self.power_balance_assets.clear();
                                self.power_balance_asset_error = Some(format!(
                                    "XUI18 portable asset admission rejected: {error:?}"
                                ));
                                break;
                            }
                        }
                    }
                    self.power_balance_runtime = runtime;
                }
                Err(error) => {
                    self.power_balance_asset_error = Some(format!(
                        "XUI18 portable asset admission unavailable because runtime admission failed: {error}"
                    ));
                    self.power_balance_runtime_error = Some(error);
                }
            }
        }
        self.refresh_intelligence_companion(world_frame, intelligence_inputs);
        self.refresh_occupation_surface(world_frame, occupation_inputs);
        let Some(pass) = prepared_frame.authenticated_ui_pass.as_ref() else {
            return Ok(SelectedPresentUiOutcome::Omitted(
                "complete authenticated selected-Present UI-pass carrier is absent".into(),
            ));
        };
        if pass.target.prepared_target != replay_core::PreparedRenderTarget::SwapchainBackBuffer {
            return Err(
                "authenticated selected-Present UI pass does not target SwapchainBackBuffer".into(),
            );
        }
        let variant_key =
            match exact_variant_admission_key(world_frame, pass, gfx.width, gfx.height) {
                Ok(key) => key,
                Err(reason) => return Ok(SelectedPresentUiOutcome::Omitted(reason)),
            };
        if self
            .variant_key
            .as_ref()
            .is_some_and(|admitted| admitted != &variant_key)
        {
            return Ok(SelectedPresentUiOutcome::Omitted(
                "XUI25 exact UI variant changed without a new authenticated selected-Present pass"
                    .into(),
            ));
        }
        self.variant_key = Some(variant_key);

        let (xui15_output, xui15_unavailable_reason) =
            match self.production_ui_renderer.as_ref() {
                Some(renderer) => match renderer.compose_xui15_frame(
                    world_frame,
                    pov_tag,
                    gfx.width as usize,
                    gfx.height as usize,
                ) {
                    Ok(output) => (output, None),
                    Err(error) => (
                        None,
                        Some(format!("XUI15 production surface rejected: {error:?}")),
                    ),
                },
                None => (
                    None,
                    Some("XUI15 production renderer is feature-local unavailable".into()),
                ),
            };
        let xui15_surface_drawn = xui15_output.is_some();

        // A34 and this pass are independently joined to the same selected
        // Present by replay_core: build_decision_panel_exact rejects any d87
        // receipt that differs from production_render.selected_present, while
        // exact_variant_admission_key above rejects a pass not carried by this
        // production frame. The pass is the complete ordered pixel operation
        // frontier for that Present, so painting DecisionPanelBinding again
        // would double-submit the panel and break exact replay. Consume the
        // heterogeneous model here as the typed companion to those pixels.
        let decision_panel_item_count = self
            .selected_pov_runtime
            .snapshot()
            .decision_panel
            .value
            .as_ref()
            .map(|panel| panel.items.len());
        let power_balance_item_count = self
            .power_balance_runtime_error
            .is_none()
            .then_some(self.power_balance_runtime.len());
        let intelligence_panel_count = self.intelligence_error.is_none().then_some(
            self.intelligence_panel_count
                .expect("successful XUI14 admission must retain its panel count"),
        );
        let intelligence_unavailable_reason = self.intelligence_error.clone();
        let snapshot = self.selected_pov_runtime.snapshot();
        let agency_upgrade_count = snapshot
            .agency_controller
            .value
            .as_ref()
            .map(|summary| summary.ordered_upgrade_ids.len());
        let cipher_target_count = snapshot
            .cipher_controller
            .value
            .as_ref()
            .map(|summary| summary.ordered_targets.len());
        let trade_license_count = snapshot.trade.value.as_ref().and_then(|trade| match trade {
            game_ui::trade_state::AdmittedTradeScreen::Visible(presentation) => {
                match &presentation.production_license_agreements {
                    game_ui::trade_state::TradeData::Available(rows) => Some(rows.len()),
                    game_ui::trade_state::TradeData::Unavailable(_) => None,
                }
            }
            game_ui::trade_state::AdmittedTradeScreen::Unavailable(_) => None,
        });
        let occupation_state_count = self.occupation_state_count;
        let occupation_unavailable_reason = self.occupation_error.clone();
        let battleplan_panel = self
            .selected_pov_runtime
            .snapshot()
            .battleplan_panel
            .value
            .as_ref();
        let battleplan_order_count = battleplan_panel.map(|panel| panel.orders.len());
        let battleplan_selected_order_present =
            battleplan_panel.map(|panel| panel.selected_order.is_some());
        let typed_consumption = self.selected_present_typed_consumption(world_frame, pass);

        let playback_context = game_ui::selected_present::SelectedPresentPlaybackContext {
            frame_hour: world_frame.hour_index(),
            pov_country_or_global: pov_tag.unwrap_or("global"),
        };

        let prepared = match game_ui::selected_present::prepare_selected_present_ui_pass(
            &self.resolver,
            &self.registry,
            pass,
            prepared_frame.m02_combat.as_ref(),
            runtime_text,
            playback_context,
        ) {
            Ok(prepared) => prepared,
            Err(error)
                if matches!(
                    error.kind,
                    game_ui::ErrorKind::Missing
                        | game_ui::ErrorKind::UnsupportedDdsFormat(_)
                        | game_ui::ErrorKind::UnsupportedTextureExtension(_)
                ) =>
            {
                return Ok(SelectedPresentUiOutcome::Omitted(format!(
                    "selected-Present UI feature unavailable: {}",
                    error.message
                )));
            }
            Err(error) => return Err(error.into()),
        };
        let rtv = gfx.rtv()?.as_raw();
        let has_m02 = prepared.operations.iter().any(|operation| {
            matches!(
                operation,
                game_ui::selected_present::PreparedUiPassOperation::M02 { .. }
            )
        });
        let mut m02_plan = if has_m02 {
            Some(unsafe {
                self.m02.prepare(
                    game_resolver,
                    rtv,
                    prepared_frame
                        .m02_combat
                        .as_ref()
                        .ok_or("prepared UI pass lost its M02 carrier")?,
                )?
            })
        } else {
            None
        };

        // Exact pipeline commands are also fully preflighted before the first
        // selected-Present operation writes the app target.
        for operation in &prepared.operations {
            if let game_ui::selected_present::PreparedUiPassOperation::ExactPipeline { command } =
                operation
            {
                let replay_core::HudCommandExecution::ExactPipeline { draw } = &command.execution
                else {
                    return Err("prepared exact UI command changed execution family".into());
                };
                unsafe {
                    self.m02.preflight_exact_ui_draw(
                        game_resolver,
                        rtv,
                        &draw.exact_draw,
                        &command.ordered_effective_vfs_used_content,
                    )?;
                }
            }
        }

        // The normal country-trade root sits below the authenticated
        // selected-Present frontier (topbar, tooltip, and popup operations).
        // Its transparent authored canvas is fully composed before this first
        // target write, then the recorded frontier retains final ordering.
        if let Some(output) = xui15_output.as_ref() {
            self.image.set_rgba(gfx, &output.image)?;
            self.image.draw(gfx)?;
        }

        let mut m02_draw_count = 0usize;
        for operation in &prepared.operations {
            match operation {
                game_ui::selected_present::PreparedUiPassOperation::M02 { output, .. } => {
                    let plan = m02_plan
                        .as_mut()
                        .ok_or("selected-Present M02 operation has no prepared plan")?;
                    unsafe {
                        self.m02.execute_next(game_resolver, rtv, plan, output)?;
                    }
                    m02_draw_count += 1;
                }
                game_ui::selected_present::PreparedUiPassOperation::ExactPipeline { command } => {
                    let replay_core::HudCommandExecution::ExactPipeline { draw } =
                        &command.execution
                    else {
                        return Err("prepared exact UI command changed execution family".into());
                    };
                    unsafe {
                        self.m02
                            .execute_exact_ui_draw(game_resolver, rtv, &draw.exact_draw)?;
                    }
                }
                game_ui::selected_present::PreparedUiPassOperation::InterpretedPaint {
                    image,
                    ..
                } => {
                    // The layer was fully rendered during the prepare
                    // preflight; upload the proven pixels instead of decoding
                    // and rasterizing the command a second time.
                    self.image.set_rgba(gfx, image)?;
                    self.image.draw(gfx)?;
                }
            }
        }
        if let Some(plan) = m02_plan {
            map_viewer::NativeM02OverlayExecutor::finish(plan)?;
        }
        Ok(SelectedPresentUiOutcome::Executed {
            operation_count: prepared.operations.len(),
            m02_draw_count,
            decision_panel_item_count,
            power_balance_item_count,
            intelligence_panel_count,
            intelligence_unavailable_reason,
            agency_upgrade_count,
            cipher_target_count,
            trade_license_count,
            xui15_surface_drawn,
            xui15_unavailable_reason,
            occupation_state_count,
            occupation_unavailable_reason,
            battleplan_order_count,
            battleplan_selected_order_present,
            typed_consumption,
        })
    }

    fn selected_present_typed_consumption(
        &self,
        world_frame: &WorldFrame,
        pass: &replay_core::AuthenticatedUiPass,
    ) -> SelectedPresentTypedConsumption {
        use game_ui::combat_detail_losses::Xui20SurfaceState;
        use game_ui::trade_state::{AdmittedTradeScreen, TradeData};

        let snapshot = self.selected_pov_runtime.snapshot();
        let mut feature_local_unavailable = Vec::new();
        for (feature, reason) in [
            ("A30_AGENCY", snapshot.agency_controller.unavailable_reason.as_deref()),
            ("A30_CIPHER", snapshot.cipher_controller.unavailable_reason.as_deref()),
            ("A31_FOCUS", snapshot.national_focus.unavailable_reason.as_deref()),
            ("A36_ABILITIES", snapshot.active_abilities.unavailable_reason.as_deref()),
            ("A39_CHARACTERS", snapshot.characters.unavailable_reason.as_deref()),
            ("A40_DESIGNER", snapshot.selected_designer.unavailable_reason.as_deref()),
            ("A40_PEACE", snapshot.peace_conference.unavailable_reason.as_deref()),
            ("D54_ORDERS_GROUP", snapshot.orders_group.unavailable_reason.as_deref()),
            ("XUI20_COMBAT", snapshot.combat_detail.unavailable_reason.as_deref()),
            ("XUI21_EVENTS", snapshot.fired_events.unavailable_reason.as_deref()),
            (
                "X10_DIRECT_TRANSITIONS",
                snapshot.direct_transitions.unavailable_reason.as_deref(),
            ),
        ] {
            if let Some(reason) = reason {
                feature_local_unavailable.push(format!("{feature}:{reason}"));
            }
        }
        if let Some(reason) = self.special_project_breakthrough_error.as_deref() {
            feature_local_unavailable.push(format!("X9_BREAKTHROUGHS:{reason}"));
        }
        let resolved_tooltip_instance_count = match world_frame.resolved_tooltip_exact() {
            view if view.status == replay_core::ViewStatus::Present => match view.frame.as_ref() {
                Some(frame) => Some(frame.instances.len()),
                None => {
                    feature_local_unavailable.push(
                        "A26_RESOLVED_TOOLTIP:Present view omitted exact frame".into(),
                    );
                    None
                }
            },
            view => {
                feature_local_unavailable.push(format!(
                    "A26_RESOLVED_TOOLTIP:{:?}",
                    view.status
                ));
                None
            }
        };

        let disrupted_supply_exact_row_count =
            match world_frame.country_disrupted_supply_exact() {
                view if view.status == replay_core::ViewStatus::Present => match &view.rows {
                    replay_core::Leaf::Measured(rows) => Some(rows.len()),
                    _ => {
                        feature_local_unavailable.push(
                            "A6_DISRUPTED_SUPPLY:Present view omitted measured exact rows".into(),
                        );
                        None
                    }
                },
                view => {
                    feature_local_unavailable.push(format!(
                        "A6_DISRUPTED_SUPPLY:{:?}",
                        view.status
                    ));
                    None
                }
            };

        let foreign_homebase_exact_row_count = match world_frame.foreign_homebase_exact() {
            view if view.status == replay_core::ViewStatus::Present => match &view.rows {
                replay_core::Leaf::Measured(rows) => Some(rows.len()),
                _ => {
                    feature_local_unavailable.push(
                        "FOREIGN_HOMEBASE:Present view omitted measured exact rows".into(),
                    );
                    None
                }
            },
            view => {
                feature_local_unavailable.push(format!(
                    "FOREIGN_HOMEBASE:{:?}",
                    view.status
                ));
                None
            }
        };

        let operative_network_assignment_count =
            match world_frame.diplomacy().operative_network_assignments() {
                replay_core::Leaf::Measured(rows) => Some(rows.len()),
                replay_core::Leaf::Absent => {
                    feature_local_unavailable
                        .push("A30_OPERATIVE_NETWORK_ASSIGNMENTS:DomainAbsent".into());
                    None
                }
                replay_core::Leaf::Unobtained => {
                    feature_local_unavailable.push(
                        "A30_OPERATIVE_NETWORK_ASSIGNMENTS:legacy or incomplete INGR membership"
                            .into(),
                    );
                    None
                }
            };

        let (delayed_event_lifecycle_count, delayed_event_lifecycle_complete) =
            match (
                world_frame.delayed_event_lifecycle_status(),
                &world_frame.delayed_event_lifecycle().events,
                &world_frame.delayed_event_lifecycle().history_complete,
            ) {
                (
                    replay_core::ViewStatus::Present,
                    replay_core::Leaf::Measured(rows),
                    replay_core::Leaf::Measured(true),
                ) => (Some(rows.len()), Some(true)),
                (status, _, _) => {
                    feature_local_unavailable.push(format!(
                        "A19_DELAYED_EVENT_LIFECYCLE:{status:?}:complete history required"
                    ));
                    (None, None)
                }
            };

        let exact_weather_rows = world_frame.weather_by_region();
        let exact_weather_complete = exact_weather_rows.len() == 305
            && exact_weather_rows.values().all(|row| {
                row.rain_light_active.is_measured()
                    && row.rain_heavy_active.is_measured()
                    && row.snow_active.is_measured()
                    && row.blizzard_active.is_measured()
                    && row.sandstorm_active.is_measured()
                    && row.arctic_water_active.is_measured()
                    && row.transition_expiry_game_hour.is_measured()
            });
        let exact_weather_region_count = match world_frame.weather_status() {
            replay_core::ViewStatus::Present if exact_weather_complete => {
                Some(exact_weather_rows.len())
            }
            status => {
                feature_local_unavailable.push(format!(
                    "D24_EXACT_WEATHER:{status:?}:exact 305-region named-flag census required"
                ));
                None
            }
        };

        let (exact_ace_owner_count, exact_ace_row_count) =
            match world_frame.ace_rosters_exact_status() {
                replay_core::ViewStatus::Present
                    if world_frame
                        .ace_rosters_exact()
                        .values()
                        .all(|view| view.status == replay_core::ViewStatus::Present) =>
                {
                    (
                        Some(world_frame.ace_rosters_exact().len()),
                        Some(
                            world_frame
                                .ace_rosters_exact()
                                .values()
                                .map(|view| view.aces.len())
                                .sum(),
                        ),
                    )
                }
                status => {
                    feature_local_unavailable.push(format!(
                        "D46_EXACT_ACE_ROSTERS:{status:?}:complete owner coverage required"
                    ));
                    (None, None)
                }
            };

        let exact_sunk_convoy_history_count =
            match &world_frame.naval_air_combat().sunk_convoy_history_exact {
                replay_core::Leaf::Measured(history)
                    if history.production_activation_authorized => Some(history.rows.len()),
                replay_core::Leaf::Measured(_) => {
                    feature_local_unavailable.push(
                        "B4_EXACT_SUNK_CONVOY_HISTORY:production activation unauthorized".into(),
                    );
                    None
                }
                replay_core::Leaf::Absent => {
                    feature_local_unavailable
                        .push("B4_EXACT_SUNK_CONVOY_HISTORY:DomainAbsent".into());
                    None
                }
                replay_core::Leaf::Unobtained => {
                    feature_local_unavailable
                        .push("B4_EXACT_SUNK_CONVOY_HISTORY:DomainUnavailable".into());
                    None
                }
            };
        let counter_binding = game_ui::binding_counters::counter_binding_for_selected_present(
            world_frame,
            self.selected_pov_runtime.snapshot().pov_country.as_deref(),
            pass,
        );
        let (
            counter_hud_authenticated_owner_count,
            counter_hud_authenticated_command_count,
            counter_hud_land_content_count,
            counter_hud_naval_content_count,
            counter_hud_air_content_count,
            counter_exact_supply_row_count,
            counter_exact_organisation_bar_row_count,
            counter_exact_order_kind_row_count,
            counter_evaluated_low_supply_row_count,
            counter_evaluated_dig_in_row_count,
            counter_evaluated_training_row_count,
            counter_evaluated_invalid_orders_row_count,
            counter_evaluated_attrition_row_count,
            counter_exact_active_ability_row_count,
        ) = match counter_binding {
            Ok(binding) => {
                let land_coverage = binding.land_field_coverage.as_ref();
                (
                    Some(binding.authenticated_owner_count),
                    Some(binding.authenticated_command_count),
                    binding.land_contents.as_ref().map(Vec::len),
                    binding.naval_contents.as_ref().map(Vec::len),
                    binding.air_contents.as_ref().map(Vec::len),
                    land_coverage.map(|coverage| coverage.exact_supply_rows),
                    land_coverage.map(|coverage| coverage.exact_organisation_bar_rows),
                    land_coverage.map(|coverage| coverage.exact_order_kind_rows),
                    land_coverage.map(|coverage| coverage.evaluated_low_supply_rows),
                    land_coverage.map(|coverage| coverage.evaluated_dig_in_rows),
                    land_coverage.map(|coverage| coverage.evaluated_training_rows),
                    land_coverage.map(|coverage| coverage.evaluated_invalid_orders_rows),
                    land_coverage.map(|coverage| coverage.evaluated_attrition_rows),
                    land_coverage.map(|coverage| coverage.exact_active_ability_rows),
                )
            }
            Err(reason) => {
                feature_local_unavailable.push(format!("CAP6_COUNTER_HUD:{reason}"));
                (
                    None, None, None, None, None, None, None, None, None, None, None, None, None,
                    None,
                )
            }
        };
        let deployment_queue = deployment_queue_consumption(
            snapshot.deployment_queue.value.as_ref(),
            snapshot.deployment_queue.unavailable_reason.as_deref(),
            snapshot.pov_country.as_deref(),
        );
        if let Some(reason) = deployment_queue.unavailable_reason.as_deref() {
            feature_local_unavailable.push(format!("X4_DEPLOYMENT_QUEUE:{reason}"));
        }
        let mut receipt = SelectedPresentTypedConsumption {
            feature_local_unavailable,
            national_focus_node_count: snapshot
                .national_focus
                .value
                .as_ref()
                .map(|model| model.nodes.len()),
            active_ability_count: snapshot
                .active_abilities
                .value
                .as_ref()
                .map(Vec::len),
            character_leader_count: snapshot
                .characters
                .value
                .as_ref()
                .map(|model| model.leaders.len()),
            character_ace_count: snapshot
                .characters
                .value
                .as_ref()
                .map(|model| model.aces.len()),
            selected_designer_record_count: snapshot
                .selected_designer
                .value
                .as_ref()
                .map(|model| model.ordered().len()),
            peace_conference_active: snapshot
                .peace_conference
                .value
                .as_ref()
                .map(|model| model.active()),
            orders_group_manager_complete: snapshot
                .orders_group
                .value
                .as_ref()
                .map(|model| model.manager_complete),
            orders_group_selected_army_present: snapshot
                .orders_group
                .value
                .as_ref()
                .and_then(|model| model.selected_army_present),
            fired_event_count: snapshot
                .fired_events
                .value
                .as_ref()
                .map(Vec::len),
            fired_event_popup_count: snapshot.fired_events.value.as_ref().map(|events| {
                events.iter().filter(|event| event.popup.is_some()).count()
            }),
            direct_transition_count: snapshot
                .direct_transitions
                .value
                .as_ref()
                .map(|history| history.transition_count),
            special_project_breakthrough_count: self
                .special_project_breakthrough_error
                .is_none()
                .then_some(self.special_project_breakthroughs.len()),
            resolved_tooltip_instance_count,
            disrupted_supply_exact_row_count,
            foreign_homebase_exact_row_count,
            operative_network_assignment_count,
            delayed_event_lifecycle_count,
            delayed_event_lifecycle_complete,
            deployment_queue_conveyor_count: deployment_queue.conveyor_count,
            deployment_queue_line_count: deployment_queue.line_count,
            deployment_queue_equipment_in_transit_count: deployment_queue
                .equipment_in_transit_count,
            exact_weather_region_count,
            exact_ace_owner_count,
            exact_ace_row_count,
            exact_sunk_convoy_history_count,
            counter_hud_authenticated_owner_count,
            counter_hud_authenticated_command_count,
            counter_hud_land_content_count,
            counter_hud_naval_content_count,
            counter_hud_air_content_count,
            counter_exact_supply_row_count,
            counter_exact_organisation_bar_row_count,
            counter_exact_order_kind_row_count,
            counter_evaluated_low_supply_row_count,
            counter_evaluated_dig_in_row_count,
            counter_evaluated_training_row_count,
            counter_evaluated_invalid_orders_row_count,
            counter_evaluated_attrition_row_count,
            counter_exact_active_ability_row_count,
            ..Default::default()
        };

        if let Some(combat) = snapshot.combat_detail.value.as_ref() {
            let surfaces = [
                &combat.land_active,
                &combat.land_side_detail,
                &combat.land_ended_lifecycle,
                &combat.land_terminal,
                &combat.naval_active,
                &combat.naval_terminal,
                &combat.air_history,
                &combat.air_region_history,
                &combat.convoy_history,
                &combat.convoy_terminal,
                &combat.combat_log_families,
                &combat.global_cross_family_order,
                &combat.war_pair_history,
                &combat.war_overview_exact,
            ];
            let admitted = surfaces.iter().filter_map(|surface| match surface {
                Xui20SurfaceState::Admitted { row_count } => Some(*row_count),
                _ => None,
            });
            let counts = admitted.collect::<Vec<_>>();
            receipt.combat_detail_admitted_surface_count = Some(counts.len());
            receipt.combat_detail_admitted_row_count = Some(counts.into_iter().sum());
        }

        match snapshot.trade.value.as_ref() {
            Some(AdmittedTradeScreen::Visible(trade)) => {
                receipt.trade_resource_pool_count = trade_data_len(&trade.resource_pools);
                receipt.trade_resource_count = trade_data_len(&trade.resource_cards);
                receipt.trade_deal_count = trade_data_len(&trade.deals);
                receipt.trade_market_stockpile_count = trade_data_len(&trade.market_stockpile);
                for (family, data) in [
                    ("resource_pools", trade_data_issue(&trade.resource_pools)),
                    ("resources", trade_data_issue(&trade.resource_cards)),
                    ("deals", trade_data_issue(&trade.deals)),
                    ("market_stockpile", trade_data_issue(&trade.market_stockpile)),
                    (
                        "production_licenses",
                        trade_data_issue(&trade.production_license_agreements),
                    ),
                ] {
                    if let Some(issue) = data {
                        receipt
                            .feature_local_unavailable
                            .push(format!("XUI15_TRADE_{family}:{issue:?}"));
                    }
                }
            }
            Some(AdmittedTradeScreen::Unavailable(reason)) => receipt
                .feature_local_unavailable
                .push(format!("XUI15_TRADE:{reason:?}")),
            None => {
                if let Some(reason) = snapshot.trade.unavailable_reason.as_deref() {
                    receipt
                        .feature_local_unavailable
                        .push(format!("XUI15_TRADE:{reason}"));
                }
            }
        }

        fn trade_data_len<T>(data: &TradeData<Vec<T>>) -> Option<usize> {
            match data {
                TradeData::Available(rows) => Some(rows.len()),
                TradeData::Unavailable(_) => None,
            }
        }

        fn trade_data_issue<T>(
            data: &TradeData<Vec<T>>,
        ) -> Option<game_ui::trade_state::TradeDataIssue> {
            match data {
                TradeData::Available(_) => None,
                TradeData::Unavailable(issue) => Some(*issue),
            }
        }

        receipt
    }

    fn refresh_intelligence_companion(
        &mut self,
        world_frame: &WorldFrame,
        inputs: IntelligenceUiSupplementalInputs<'_>,
    ) {
        let Some(trace) = inputs.trace else {
            self.intelligence_error = Some(
                "XUI14 semantic supplemental input is absent; feature omitted".into(),
            );
            return;
        };
        let companion = match &world_frame.diplomacy().intelligence_controller_companion {
            replay_core::Leaf::Measured(companion) => companion,
            replay_core::Leaf::Absent => {
                self.intelligence_error = Some(
                    "XUI14 selected-Present controller companion is absent".into(),
                );
                return;
            }
            replay_core::Leaf::Unobtained => {
                self.intelligence_error = Some(
                    "XUI14 selected-Present controller companion is unavailable".into(),
                );
                return;
            }
        };
        let Some(render) = world_frame.production_render() else {
            self.intelligence_error = Some(
                "XUI14 selected-Present controller companion has no production frame".into(),
            );
            return;
        };
        match game_ui::intelligence_ui::admit_intelligence_ui_with_selected_present_companion(
            trace,
            companion,
            &render.selected_present,
        ).and_then(|admitted| admitted.render_panels()) {
            Ok(panels) => {
                self.intelligence_panel_count = Some(panels.len());
                self.intelligence_error = None;
            }
            Err(error) => {
                self.intelligence_error = Some(format!(
                    "XUI14 selected-Present companion admission rejected: {}",
                    error.message
                ));
            }
        }
    }

    fn refresh_occupation_surface(
        &mut self,
        world_frame: &WorldFrame,
        inputs: OccupationUiSupplementalInputs<'_>,
    ) {
        let Some(request) = inputs.request else {
            return;
        };
        if !std::ptr::eq(request.frame, world_frame) {
            self.occupation_error = Some(
                "XUI16 production surface frame is not the selected-Present world frame".into(),
            );
            return;
        }
        let Some(renderer) = self.production_ui_renderer.as_ref() else {
            return;
        };
        match renderer.admit_xui16_surface(request) {
            Ok(surface) => {
                self.occupation_state_count = Some(surface.states().len());
                self.occupation_error = None;
            }
            Err(error) => {
                self.occupation_error = Some(format!(
                    "XUI16 production surface admission rejected: {error:?}"
                ));
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DeploymentQueueConsumption {
    conveyor_count: Option<usize>,
    line_count: Option<usize>,
    equipment_in_transit_count: Option<usize>,
    unavailable_reason: Option<String>,
}

impl DeploymentQueueConsumption {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            conveyor_count: None,
            line_count: None,
            equipment_in_transit_count: None,
            unavailable_reason: Some(reason.into()),
        }
    }
}

/// Consume only the deployment model published by `SelectedPovRuntime` for
/// this selected Present. The product receipt counts admitted hierarchy rows;
/// it never derives queue amounts or converts an unavailable leaf into zero.
fn deployment_queue_consumption(
    value: Option<&AdmittedDeploymentQueue>,
    feature_unavailable_reason: Option<&str>,
    selected_pov: Option<&str>,
) -> DeploymentQueueConsumption {
    if value.is_some() && feature_unavailable_reason.is_some() {
        return DeploymentQueueConsumption::unavailable(
            "presentation feature carried both a value and an unavailable reason",
        );
    }
    match value {
        Some(AdmittedDeploymentQueue::Available(queue)) => {
            let Some(selected_pov) = selected_pov else {
                return DeploymentQueueConsumption::unavailable(
                    "admitted hierarchy has no selected POV identity",
                );
            };
            if queue.country_tag != selected_pov {
                return DeploymentQueueConsumption::unavailable(format!(
                    "admitted hierarchy country {:?} does not match selected POV {:?}",
                    queue.country_tag, selected_pov
                ));
            }
            let line_count = queue
                .conveyors
                .iter()
                .map(|conveyor| conveyor.lines.len())
                .sum();
            let equipment_in_transit_count = queue
                .conveyors
                .iter()
                .flat_map(|conveyor| conveyor.lines.iter())
                .map(|line| line.equipment_in_transit.len())
                .sum();
            DeploymentQueueConsumption {
                conveyor_count: Some(queue.conveyors.len()),
                line_count: Some(line_count),
                equipment_in_transit_count: Some(equipment_in_transit_count),
                unavailable_reason: None,
            }
        }
        Some(AdmittedDeploymentQueue::Unavailable(reason)) => {
            DeploymentQueueConsumption::unavailable(format!("{reason:?}"))
        }
        None => DeploymentQueueConsumption::unavailable(
            feature_unavailable_reason
                .unwrap_or("presentation feature omitted without an unavailable reason"),
        ),
    }
}

fn exact_variant_admission_key(
    frame: &WorldFrame,
    pass: &replay_core::AuthenticatedUiPass,
    client_width: u32,
    client_height: u32,
) -> std::result::Result<DisplayVariantKey, String> {
    let Some(render) = frame.production_render() else {
        return Err("selected-Present UI pass lacks the current production frame".into());
    };
    let Some(carried_pass) = render.authenticated_ui_pass.as_ref() else {
        return Err("selected-Present UI pass is absent from the current production frame".into());
    };
    if carried_pass.pass_identity_sha256 != pass.pass_identity_sha256
        || carried_pass.update_index != pass.update_index
    {
        return Err(
            "selected-Present UI pass does not belong to the current production frame".into(),
        );
    }
    let session = frame.session_metadata();
    let key = exact_variant_key(
        session.display_variant.measured(),
        session.localisation_language.measured().map(String::as_str),
        render.authority.recorded_input_variant,
        PixelExtent {
            width: pass.target.width,
            height: pass.target.height,
        },
        PixelExtent {
            width: client_width,
            height: client_height,
        },
    )?;
    let contract = ExactKeyVariantContract::admit(production_ui_variant_key())
        .map_err(|error| format!("XUI25 product exact-key contract is invalid: {error:?}"))?;
    contract
        .select(&key)
        .map_err(|error| format!("XUI25 selected-Present variant is unsupported: {error:?}"))?;
    Ok(key)
}

fn exact_variant_key(
    display: Option<&replay_core::world::DisplayVariantMetadata>,
    language: Option<&str>,
    recorded_input: Option<RecordedInputVariant>,
    target_extent: PixelExtent,
    client_extent: PixelExtent,
) -> std::result::Result<DisplayVariantKey, String> {
    let Some(display) = display else {
        return Err("XUI25 exact UI variant lacks session.display_variant".into());
    };
    let Some(language) = language else {
        return Err("XUI25 exact UI variant lacks session.localisation_language".into());
    };
    let Some(recorded_input) = recorded_input else {
        return Err("XUI25 exact UI variant lacks recorded input family".into());
    };
    let input = match recorded_input {
        RecordedInputVariant::MouseKeyboard => InputVariant::MouseKeyboard,
        RecordedInputVariant::Keyboard => InputVariant::Keyboard,
        RecordedInputVariant::Controller => InputVariant::Controller,
    };
    let key = match DisplayVariantKey::from_session_metadata(display, language, input) {
        Ok(key) => key,
        Err(error) => return Err(format!("XUI25 exact UI variant is unsupported: {error:?}")),
    };
    if key.physical_viewport != target_extent || key.physical_viewport != client_extent {
        return Err(format!(
            "XUI25 exact UI variant mismatch: recorded {:?}, pass {:?}, client {:?}",
            key.physical_viewport, target_extent, client_extent
        ));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_route_source_has_no_flattened_product_ui_fallback() {
        let source = include_str!("ui_runtime.rs");
        let legacy_type = ["struct Product", "UiRuntime"].concat();
        let legacy_compose = ["compose_hud", "_frame("].concat();
        assert!(!source.contains(&legacy_type));
        assert!(!source.contains(&legacy_compose));
        assert!(source.contains(".battleplan_panel\n            .value"));
    }

    #[test]
    fn x4_receipt_distinguishes_measured_empty_absent_and_unobtained() {
        use game_ui::deployment_queue_state::{
            DeploymentConveyorPresentation, DeploymentEquipmentPresentation,
            DeploymentLinePresentation, DeploymentQueuePresentation,
            DeploymentQueueUnavailable,
        };

        let admitted = AdmittedDeploymentQueue::Available(DeploymentQueuePresentation {
            country_tag: "GER".into(),
            conveyors: vec![DeploymentConveyorPresentation {
                ordinal: 0,
                conveyor_id: 41,
                name: "1. Panzer-Division".into(),
                template_id: 17,
                amount: 1,
                location: 64,
                priority: 2,
                role: "armor".into(),
                closed: false,
                lines: vec![DeploymentLinePresentation {
                    ordinal: 0,
                    line_id: 73,
                    name_order: replay_core::Leaf::Measured(0),
                    amount: 1,
                    training: replay_core::Leaf::Measured(0.25),
                    max_training: replay_core::Leaf::Measured(1.0),
                    manpower_have: replay_core::Leaf::Measured(5_000),
                    manpower_need: replay_core::Leaf::Measured(10_000),
                    max_manpower: replay_core::Leaf::Measured(10_000),
                    equipment_in_transit: vec![DeploymentEquipmentPresentation {
                        ordinal: 0,
                        equipment_id: 91,
                        archetype: replay_core::Leaf::Measured("light_tank_equipment".into()),
                        amount: 20.0,
                    }],
                }],
            }],
        });
        let measured = deployment_queue_consumption(Some(&admitted), None, Some("GER"));
        assert_eq!(measured.conveyor_count, Some(1));
        assert_eq!(measured.line_count, Some(1));
        assert_eq!(measured.equipment_in_transit_count, Some(1));
        assert_eq!(measured.unavailable_reason, None);

        let empty = AdmittedDeploymentQueue::Available(DeploymentQueuePresentation {
            country_tag: "GER".into(),
            conveyors: Vec::new(),
        });
        let measured_empty = deployment_queue_consumption(Some(&empty), None, Some("GER"));
        assert_eq!(measured_empty.conveyor_count, Some(0));
        assert_eq!(measured_empty.line_count, Some(0));
        assert_eq!(measured_empty.equipment_in_transit_count, Some(0));
        assert_eq!(measured_empty.unavailable_reason, None);

        let absent = AdmittedDeploymentQueue::Unavailable(
            DeploymentQueueUnavailable::NotRecorded,
        );
        let absent = deployment_queue_consumption(Some(&absent), None, Some("GER"));
        assert_eq!(absent.conveyor_count, None);
        assert_eq!(absent.unavailable_reason.as_deref(), Some("NotRecorded"));

        let unavailable = AdmittedDeploymentQueue::Unavailable(
            DeploymentQueueUnavailable::DomainUnavailable,
        );
        let unavailable = deployment_queue_consumption(Some(&unavailable), None, Some("GER"));
        assert_eq!(unavailable.conveyor_count, None);
        assert_eq!(
            unavailable.unavailable_reason.as_deref(),
            Some("DomainUnavailable")
        );

        let unobtained = AdmittedDeploymentQueue::Unavailable(
            DeploymentQueueUnavailable::CapturedButUnobtained,
        );
        let unobtained = deployment_queue_consumption(Some(&unobtained), None, Some("GER"));
        assert_eq!(unobtained.conveyor_count, None);
        assert_eq!(
            unobtained.unavailable_reason.as_deref(),
            Some("CapturedButUnobtained")
        );
        assert_ne!(
            absent.unavailable_reason.as_deref(),
            unobtained.unavailable_reason.as_deref()
        );
        assert_ne!(
            unavailable.unavailable_reason.as_deref(),
            unobtained.unavailable_reason.as_deref()
        );
    }

    #[test]
    fn x4_receipt_rejects_pov_identity_mismatch() {
        let queue = AdmittedDeploymentQueue::Available(
            game_ui::deployment_queue_state::DeploymentQueuePresentation {
                country_tag: "GER".into(),
                conveyors: Vec::new(),
            },
        );
        let receipt = deployment_queue_consumption(Some(&queue), None, Some("ITA"));
        assert_eq!(receipt.conveyor_count, None);
        assert!(receipt
            .unavailable_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("does not match selected POV")));
    }

    fn display() -> replay_core::world::DisplayVariantMetadata {
        replay_core::world::DisplayVariantMetadata {
            physical_width: 1920,
            physical_height: 1080,
            min_gui_width: 1280,
            min_gui_height: 720,
            gui_scale_f32_bits: 1.0f32.to_bits(),
            refresh_rate_hz: 60,
            max_refresh_rate_hz: Some(60),
            fullscreen: false,
            borderless: true,
            display_mode: "borderless_fullscreen".into(),
            settings_file_sha256: "11".repeat(32),
        }
    }

    #[test]
    fn xui25_exact_admission_checks_every_recorded_dimension() {
        let extent = PixelExtent {
            width: 1920,
            height: 1080,
        };
        assert!(exact_variant_key(
            Some(&display()),
            Some("l_english"),
            Some(RecordedInputVariant::MouseKeyboard),
            extent,
            extent,
        )
        .is_ok());

        let mut variants = Vec::new();
        let mut value = display();
        value.physical_width += 1;
        variants.push(value);
        let mut value = display();
        value.min_gui_width += 1;
        variants.push(value);
        let mut value = display();
        value.gui_scale_f32_bits += 1;
        variants.push(value);
        let mut value = display();
        value.display_mode = "windowed".into();
        variants.push(value);
        for value in variants {
            // The selected-Present pass is the producer-authenticated exact
            // envelope. A display mutation must either change its physical
            // extent or remain represented in the exact key without being
            // silently defaulted.
            let result = exact_variant_key(
                Some(&value),
                Some("l_english"),
                Some(RecordedInputVariant::MouseKeyboard),
                extent,
                extent,
            );
            if value.physical_width != extent.width {
                assert!(result.is_err());
            } else {
                let key = result.unwrap();
                assert_eq!(key.min_gui.width, value.min_gui_width);
                assert_eq!(key.gui_scale_f32_bits, value.gui_scale_f32_bits);
                assert_eq!(
                    key.display_mode,
                    if value.display_mode == "windowed" {
                        game_ui::variant_admission::DisplayMode::Windowed
                    } else {
                        game_ui::variant_admission::DisplayMode::BorderlessFullscreen
                    }
                );
            }
        }

        let english = exact_variant_key(
            Some(&display()),
            Some("l_english"),
            Some(RecordedInputVariant::MouseKeyboard),
            extent,
            extent,
        )
        .unwrap();
        let french = exact_variant_key(
            Some(&display()),
            Some("l_french"),
            Some(RecordedInputVariant::MouseKeyboard),
            extent,
            extent,
        )
        .unwrap();
        assert_ne!(english, french);
    }

    #[test]
    fn xui25_missing_language_display_input_and_unsupported_input_omit_locally() {
        let extent = PixelExtent {
            width: 1920,
            height: 1080,
        };
        for result in [
            exact_variant_key(
                None,
                Some("l_english"),
                Some(RecordedInputVariant::MouseKeyboard),
                extent,
                extent,
            ),
            exact_variant_key(
                Some(&display()),
                None,
                Some(RecordedInputVariant::MouseKeyboard),
                extent,
                extent,
            ),
            exact_variant_key(Some(&display()), Some("l_english"), None, extent, extent),
            exact_variant_key(
                Some(&display()),
                Some("l_english"),
                Some(RecordedInputVariant::Controller),
                extent,
                extent,
            ),
        ] {
            assert!(result.is_err());
        }
    }
}
