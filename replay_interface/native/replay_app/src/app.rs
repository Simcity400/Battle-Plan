//! Product window loop: input, timeline scrub, picking, rendering.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use map_viewer::GameFileResolver;
use replay_core::{ArmyBattleplanOrderKey, ArmyBattleplanOwnerKey, ViewStatus, WorldFrame};
use winapi::um::winuser::{
    VK_ADD, VK_ESCAPE, VK_LEFT, VK_OEM_4, VK_OEM_6, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD,
    VK_OEM_PLUS, VK_RIGHT, VK_SPACE, VK_SUBTRACT,
};

use crate::bundle::MapBundle;
use crate::camera::Camera;
use crate::d3d::Gfx;
use crate::observer::{PovPicker, PovRenderStatus, PovState};
use crate::overlay::{self, Overlay};
use crate::render::{
    playback_day_night_hour_from_timeline_hour, MapRenderer, POLITICAL_COLOR_BLOCKER_BADGE,
};
use crate::timeline::{build_provider, OwnerProvider};
use crate::transport::Transport;
use crate::ui_runtime::{
    ArmyBattleplanUiSupplementalInputs, IntelligenceUiSupplementalInputs,
    OccupationUiSupplementalInputs, ProductSelectedPresentRuntime, SelectedPresentUiOutcome,
};
use crate::win32::{MouseButton, Window, WindowEvent};
use crate::{Result, TimelineMode};

// Interaction tuning constants (not visual claims).
const KEY_PAN_SCREEN_PX_PER_SEC: f64 = 900.0;
const MAX_FRAME_DT: f64 = 0.25;

const VK_A: i32 = 0x41;
const VK_B: i32 = 0x42;
const VK_C: i32 = 0x43;
const VK_D: i32 = 0x44;
const VK_M: i32 = 0x4D;
const VK_N: i32 = 0x4E;
const VK_O: i32 = 0x4F;
const VK_R: i32 = 0x52;
const VK_1: i32 = 0x31;
const VK_9: i32 = 0x39;

#[derive(Default)]
struct PanKeys {
    west: bool,
    east: bool,
}

struct DragState {
    start_cursor: (i32, i32),
    start_center_x: f64,
}

/// Product-owned army/order selection for the typed d54 battleplan companion.
///
/// The current map route has no authenticated unit/counter hit-test event, so
/// startup deliberately leaves this empty. Explicit B/M keyboard input cycles
/// only identities already admitted from the current typed frame; the render
/// path never infers a first army or order without that user action.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BattleplanSelectionState {
    selected_owner: Option<ArmyBattleplanOwnerKey>,
    selected_order_key: Option<ArmyBattleplanOrderKey>,
}

impl BattleplanSelectionState {
    pub(crate) fn select_owner(&mut self, owner: ArmyBattleplanOwnerKey) {
        if self.selected_owner != Some(owner) {
            self.selected_order_key = None;
        }
        self.selected_owner = Some(owner);
    }

    pub(crate) fn select_order(&mut self, key: ArmyBattleplanOrderKey) {
        self.selected_owner = Some(key.owner);
        self.selected_order_key = Some(key);
    }

    pub(crate) fn clear(&mut self) {
        self.selected_owner = None;
        self.selected_order_key = None;
    }

    /// Cycle only over typed owners present in the admitted d54 order view.
    /// This is explicit user input and never guesses a map/unit hit-test.
    pub(crate) fn cycle_owner(&mut self, frame: &WorldFrame) {
        let view = frame.army_battleplans();
        if view.status != ViewStatus::Present {
            self.clear();
            return;
        }
        let mut owners = view
            .orders
            .iter()
            .map(|order| order.key.owner)
            .collect::<Vec<_>>();
        owners.sort_unstable();
        owners.dedup();
        if owners.is_empty() {
            self.clear();
            return;
        }
        let next = self
            .selected_owner
            .and_then(|selected| owners.iter().position(|owner| *owner == selected))
            .map(|index| owners[(index + 1) % owners.len()])
            .unwrap_or(owners[0]);
        self.select_owner(next);
    }

    /// Cycle stable order identities under the already selected typed owner.
    pub(crate) fn cycle_order(&mut self, frame: &WorldFrame) {
        let Some(owner) = self.selected_owner else {
            return;
        };
        let view = frame.army_battleplans();
        if view.status != ViewStatus::Present {
            self.selected_order_key = None;
            return;
        }
        let orders = view
            .orders_for_owner(owner)
            .map(|order| order.key.clone())
            .collect::<Vec<_>>();
        if orders.is_empty() {
            self.selected_order_key = None;
            return;
        }
        let next = self
            .selected_order_key
            .as_ref()
            .and_then(|selected| orders.iter().position(|key| key == selected))
            .map(|index| orders[(index + 1) % orders.len()].clone())
            .unwrap_or_else(|| orders[0].clone());
        self.select_order(next);
    }

    fn supplemental_inputs(&self) -> ArmyBattleplanUiSupplementalInputs<'_> {
        ArmyBattleplanUiSupplementalInputs {
            selected_owner: self.selected_owner,
            selected_order_key: self.selected_order_key.as_ref(),
        }
    }
}

#[derive(Default)]
pub(crate) struct ProductProductionD3dRuntime {
    backend: Option<map_viewer::NativeProductionD3dBackend>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProductProductionD3dOutcome {
    pub(crate) executed_operations: usize,
    pub(crate) executed_draw_tokens: Vec<String>,
    pub(crate) presented_to_app_target: bool,
    pub(crate) receipt_sha256: String,
    pub(crate) snow_mud_proof: Option<map_viewer::SnowMudD3dExecutionProof>,
    pub(crate) occupation_secondary_proof: Option<map_viewer::OccupationSecondaryD3dExecutionProof>,
}

impl ProductProductionD3dRuntime {
    fn invalidate(&mut self) {
        // The backend owns AddRef'd app interfaces, including the current RTV.
        // Release it before ResizeBuffers drops/replaces the app-owned view.
        self.backend = None;
    }

    pub(crate) fn execute(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        resources: &std::collections::BTreeMap<String, map_viewer::ProductionMapResource>,
        frame: &map_viewer::PreparedProductionD3dFrame,
    ) -> Result<ProductProductionD3dOutcome> {
        self.execute_inner(gfx, resolver, resources, frame, false)
    }

    pub(crate) fn execute_with_snow_mud_h2(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        resources: &std::collections::BTreeMap<String, map_viewer::ProductionMapResource>,
        frame: &map_viewer::PreparedProductionD3dFrame,
    ) -> Result<ProductProductionD3dOutcome> {
        self.execute_inner(gfx, resolver, resources, frame, true)
    }

    fn execute_inner(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        resources: &std::collections::BTreeMap<String, map_viewer::ProductionMapResource>,
        frame: &map_viewer::PreparedProductionD3dFrame,
        collect_snow_mud_h2: bool,
    ) -> Result<ProductProductionD3dOutcome> {
        let target = frame
            .target
            .as_ref()
            .ok_or("production D3D frame has no authenticated MainScene target")?;
        if self
            .backend
            .as_ref()
            .is_some_and(|backend| backend.desc() != target)
        {
            self.backend = None;
        }
        if self.backend.is_none() {
            self.backend = Some(unsafe {
                map_viewer::NativeProductionD3dBackend::create_on_device(
                    gfx.device.as_raw(),
                    gfx.context.as_raw(),
                    target.clone(),
                )?
            });
        }
        let backend = self
            .backend
            .as_mut()
            .ok_or("production D3D backend was not constructed")?;
        unsafe {
            backend.attach_presentation_target(
                gfx.rtv()?.as_raw(),
                gfx.width,
                gfx.height,
                winapi::shared::dxgiformat::DXGI_FORMAT_B8G8R8A8_UNORM,
                1,
                0,
            )?;
        }
        let presented_to_app_target = frame_has_app_presentation(frame);
        let mut executor = map_viewer::NativeProductionD3dExecutor::new_with_snow_mud_h2(
            backend,
            resolver,
            resources,
            collect_snow_mud_h2,
        )?;
        let receipt = map_viewer::execute_production_d3d_frame(frame, &mut executor)?;
        let snow_mud_proof = executor.snow_mud_execution_proof().cloned();
        let occupation_secondary_proof = executor.occupation_secondary_execution_proof().cloned();
        Ok(ProductProductionD3dOutcome {
            executed_operations: receipt.ordered_draws.len(),
            executed_draw_tokens: receipt
                .ordered_draws
                .iter()
                .map(|draw| draw.draw_token.clone())
                .collect(),
            presented_to_app_target,
            receipt_sha256: receipt.deterministic_sha256,
            snow_mud_proof,
            occupation_secondary_proof,
        })
    }
}

fn frame_has_app_presentation(frame: &map_viewer::PreparedProductionD3dFrame) -> bool {
    use replay_core::{PreparedRenderTarget, StaticMapCommand};

    let static_presents = frame.ordered_static_programs.iter().any(|program| {
        program
            .ordered_operations
            .iter()
            .any(|operation| match &operation.command {
                StaticMapCommand::ClearColor { target, .. } => {
                    target.prepared_target == PreparedRenderTarget::SwapchainBackBuffer
                }
                StaticMapCommand::ClearDepthStencil { .. } => false,
                StaticMapCommand::Draw { draw } => draw
                    .output_state
                    .ordered_color_targets
                    .iter()
                    .any(|target| {
                        target.prepared_target == PreparedRenderTarget::SwapchainBackBuffer
                    }),
            })
    });
    let m03_presents = frame.m03_arrows.as_ref().is_some_and(|arrows| {
        arrows.draws.iter().any(|draw| {
            draw.draw
                .output_state
                .ordered_color_targets
                .iter()
                .any(|target| target.prepared_target == PreparedRenderTarget::SwapchainBackBuffer)
        })
    });
    static_presents
        || m03_presents
        || frame.ordered_runtime_draws.iter().any(|draw| {
            let output = match draw {
                map_viewer::PreparedRuntimeDraw::Particle { packet, .. } => &packet.output_state,
                map_viewer::PreparedRuntimeDraw::Entity { submission, .. } => {
                    &submission.output_state
                }
                map_viewer::PreparedRuntimeDraw::O04City { submission, .. } => {
                    &submission.output_state
                }
                map_viewer::PreparedRuntimeDraw::O05Building { submission, .. } => {
                    &submission.output_state
                }
            };
            output
                .ordered_color_targets
                .iter()
                .any(|target| target.prepared_target == PreparedRenderTarget::SwapchainBackBuffer)
        })
}

fn frame_has_authenticated_map_operations(frame: &map_viewer::PreparedProductionD3dFrame) -> bool {
    map_viewer::has_authenticated_d80_map_operations(frame)
}

/// Bind transport to the authenticated hour index without enumerating dates.
/// Date metadata remains behind `OwnerProvider::date_at` and is resolved only
/// by an explicit date-navigation command.
fn transport_from_provider_index(provider: &dyn OwnerProvider) -> Result<Transport> {
    Transport::with_lazy_dates(provider.hours().to_vec()).map_err(|error| error.into())
}

pub fn run(
    bundle: Option<MapBundle>,
    mode: TimelineMode,
    _game_resolver: Option<GameFileResolver>,
    product_paths: crate::install_paths::ProductPaths,
    capture_dir: Option<PathBuf>,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    water_full: bool,
) -> Result<()> {
    let debug_overlay_enabled = capture_dir.is_some();
    let production_recording = matches!(&mode, TimelineMode::Recording { .. });
    let (provider, mode_label) = build_provider(bundle.as_ref(), &mode)?;
    let (game_resolver, content_admission) =
        crate::content_compatibility::bind_provider_content(provider.as_ref(), &product_paths)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let content_manifest = provider
        .content_manifest()
        .ok_or("portable playback has no admitted content manifest after binding")?;
    // Consume the authenticated recorder payload bytes as semantic catalogs
    // during ordinary product construction.  Missing/malformed artifacts abort
    // playback; no installed-game or built-in catalog may replace them.
    let static_unit_catalogs = game_ui::static_unit_catalogs::admit_static_unit_catalogs(
        provider.static_artifact(
            game_ui::static_unit_catalogs::TEMPLATES_EQUIPMENT_ARTIFACT,
        ),
        provider.static_artifact(game_ui::static_unit_catalogs::SHIP_DEFINITIONS_ARTIFACT),
    )
    .map_err(|error| format!("admit packaged static unit catalogs: {error}"))?;
    let static_research_catalogs = Arc::new(
        game_ui::static_research::admit_static_research_catalogs(
            provider.static_artifact(replay_core::LOCALISATION_JOIN_ARTIFACT),
            provider.static_artifact(replay_core::IDEA_TECH_LOC_XREF_ARTIFACT),
            provider.static_artifact(replay_core::TECHNOLOGY_SHARING_ARTIFACT),
        )
        .map_err(|error| format!("admit packaged static research catalogs: {error}"))?,
    );
    let static_focus_definitions = Arc::new(
        replay_core::admit_static_focus_definitions(
            provider
                .static_artifact(replay_core::FOCUS_DEFS_STATIC_ARTIFACT)
                .ok_or("portable playback has no authenticated focus_defs_static artifact")?,
        )
        .map_err(|error| format!("admit packaged static focus definitions: {error}"))?,
    );
    let static_focus_catalog = Arc::new(
        game_ui::static_focus_defs::focus_catalog_from_static(&static_focus_definitions),
    );
    let mut production_map = map_viewer::ProductionMapRenderer::new(
        &game_resolver,
        &content_admission,
        content_manifest,
    )?;
    let mut reported_production_data_needs = BTreeSet::new();
    // E1 transport: play/pause, speed steps, seek, jump-to-date over the
    // recording's authenticated hour/date index. The date side is deliberately
    // lazy: startup must not enumerate every date or decode every frame. E6
    // POV: the country picker source + the fail-closed POV state object.
    let mut transport = transport_from_provider_index(provider.as_ref())?;
    let hours: Vec<i64> = transport.hours().to_vec();
    let pov_picker = provider.pov_picker();
    let mut pov = PovState::new();
    match &pov_picker {
        PovPicker::Available(choices) => eprintln!(
            "replay_app: observer POV picker: {} player countr{} (press 1-9 to observe, O to toggle)",
            choices.len(),
            if choices.len() == 1 { "y" } else { "ies" }
        ),
        PovPicker::Unavailable(reason) => {
            eprintln!("replay_app: observer POV picker unavailable: {reason}")
        }
    }
    // The state->color table is sized by the bundle: a state id outside the
    // bundle's province->state mapping can never be displayed, and the
    // palette builder skips owner rows beyond the table.
    let max_state_id = bundle.as_ref().map(MapBundle::max_state_id);

    // The first recorded frame owns the product client extent. Never turn the
    // frozen XUI25 test envelope into a runtime default.
    let initial_frame = provider
        .world_frame_at(hours[0])?
        .ok_or("initial replay frame is unavailable")?;
    if production_recording
        && initial_frame.production_render_status() != replay_core::ViewStatus::Present
    {
        return Err(
            "initial production replay frame has no authenticated production-render program"
                .into(),
        );
    }
    transport.apply_recorded_session_timing(initial_frame.session_metadata());
    let initial_display = initial_frame
        .session_metadata()
        .display_variant
        .measured()
        .ok_or("replay lacks authenticated initial display_variant")?;
    let window_width = i32::try_from(initial_display.physical_width)
        .map_err(|_| "recorded physical width exceeds the Win32 client extent")?;
    let window_height = i32::try_from(initial_display.physical_height)
        .map_err(|_| "recorded physical height exceeds the Win32 client extent")?;
    if window_width <= 0 || window_height <= 0 {
        return Err("recorded physical client extent is empty".into());
    }
    let mut window = Window::create("replay_app", window_width, window_height)?;
    let (client_w, client_h) = window.client_size()?;
    let mut gfx = Gfx::create(window.hwnd(), client_w, client_h)?;
    let mut renderer = match (bundle.as_ref(), capture_dir.as_deref(), max_state_id) {
        (Some(bundle), Some(capture_dir), Some(max_state_id)) => Some(MapRenderer::new(
            &gfx,
            bundle,
            max_state_id,
            Some(capture_dir),
            Some(&game_resolver),
            postfx_enabled,
            postfx_restore_full,
            water_full,
        )?),
        _ => {
            eprintln!(
                "replay_app: debug map renderer omitted: --bundle/--capture are evidence-only; the production D3D route will execute only when this portable frame carries a complete admitted program and SwapchainBackBuffer output"
            );
            None
        }
    };
    // The proxy overlay is an explicit capture/debug facility. Normal product
    // playback never constructs it and therefore never probes unrecorded UI or
    // font content from a non-UI-only manifest.
    let mut overlay = debug_overlay_enabled.then(|| Overlay::create(&gfx, Some(&game_resolver)));
    // Exact product playback has one HUD route: the authenticated selected-
    // Present operation stream. The older flattened semantic HUD compositor
    // is deliberately not constructed as a fallback.
    let mut camera = bundle.as_ref().map(|bundle| {
        Camera::fit(
            bundle.width,
            bundle.height,
            f64::from(gfx.width),
            f64::from(gfx.height),
        )
    });
    let mut uploaded_hour_index: Option<usize> = None;
    let mut prepared_production_d3d: Option<map_viewer::PreparedProductionD3dFrame> = None;
    let mut prepared_selected_present_ui: Option<map_viewer::PreparedProductionD3dFrame> = None;
    let mut production_map_refused_for_displayed_hour = false;
    let mut product_production_d3d = ProductProductionD3dRuntime::default();
    let mut product_selected_present_ui: Option<ProductSelectedPresentRuntime> = None;
    // No current map/unit callback owns an exact order identity. Keep startup
    // empty; explicit B/M input below selects only current typed d54 identities.
    let mut battleplan_selection = BattleplanSelectionState::default();
    // Day/night terminator shading is OFF by default, matching the game's
    // default map view (the captured CB0 gate is off); 'N' toggles it.
    let mut day_night_enabled = false;
    let mut pan_keys = PanKeys::default();
    let mut drag: Option<DragState> = None;
    let mut scrub_drag = false;
    let mut cursor: Option<(i32, i32)> = None;
    let mut closing = false;
    let mut last_frame = Instant::now();

    loop {
        let mut cycle_battleplan_owner = false;
        let mut cycle_battleplan_order = false;
        let mut clear_battleplan_selection = false;
        let (events, alive) = window.pump();
        if !alive {
            break;
        }
        let now = Instant::now();
        let dt = (now - last_frame).as_secs_f64().min(MAX_FRAME_DT);
        last_frame = now;

        let screen_w = f64::from(gfx.width);
        let screen_h = f64::from(gfx.height);
        for event in events {
            match event {
                WindowEvent::Resized { width, height } => {
                    product_production_d3d.invalidate();
                    product_selected_present_ui = None;
                    gfx.resize(width, height)?;
                    uploaded_hour_index = None;
                    if let Some(camera) = &mut camera {
                        camera.normalize(f64::from(gfx.height));
                    }
                }
                WindowEvent::KeyDown { vk } => match vk {
                    VK_ESCAPE => {
                        closing = true;
                        window.request_close();
                    }
                    VK_SPACE => {
                        transport.toggle_play();
                    }
                    VK_LEFT | VK_OEM_4 => {
                        transport.step_backward();
                    }
                    VK_RIGHT | VK_OEM_6 => {
                        transport.step_forward();
                    }
                    VK_OEM_PLUS | VK_ADD => transport.speed_up(),
                    VK_OEM_MINUS | VK_SUBTRACT => transport.speed_down(),
                    // Jump to the previous / next calendar-day boundary.
                    VK_OEM_COMMA => {
                        let date_ticks = transport.date_ticks_with(|hour| {
                            provider.date_at(hour).map_err(|error| error.to_string())
                        })?;
                        let here = transport.current_index();
                        if let Some(tick) = date_ticks.iter().rev().find(|tick| tick.index < here) {
                            transport.seek_to_index(tick.index);
                        }
                    }
                    VK_OEM_PERIOD => {
                        let date_ticks = transport.date_ticks_with(|hour| {
                            provider.date_at(hour).map_err(|error| error.to_string())
                        })?;
                        let here = transport.current_index();
                        if let Some(tick) = date_ticks.iter().find(|tick| tick.index > here) {
                            transport.seek_to_index(tick.index);
                        }
                    }
                    // E6: one-key toggle between the world view and the last
                    // observed POV; digits 1-9 pick a player country.
                    VK_O => pov.toggle(),
                    digit if (VK_1..=VK_9).contains(&digit) => {
                        if let PovPicker::Available(choices) = &pov_picker {
                            let slot = (digit - VK_1) as usize;
                            if let Some(choice) = choices.get(slot) {
                                pov.enter_pov(&choice.tag);
                            }
                        }
                    }
                    VK_R => {
                        if let Some(camera) = &mut camera {
                            camera.reset_fit(screen_w, screen_h);
                        }
                    }
                    VK_N => {
                        day_night_enabled = !day_night_enabled;
                    }
                    // Exact d54 keyboard selection. B cycles typed owners, M
                    // cycles stable orders under that owner, C clears. These
                    // identities come only from the admitted replay frame.
                    VK_B => cycle_battleplan_owner = true,
                    VK_M => cycle_battleplan_order = true,
                    VK_C => clear_battleplan_selection = true,
                    VK_A => pan_keys.west = true,
                    VK_D => pan_keys.east = true,
                    _ => {}
                },
                WindowEvent::KeyUp { vk } => match vk {
                    VK_A => pan_keys.west = false,
                    VK_D => pan_keys.east = false,
                    _ => {}
                },
                WindowEvent::MouseMove { x, y } => {
                    cursor = Some((x, y));
                    if debug_overlay_enabled && scrub_drag {
                        if let Some(fraction) =
                            overlay::scrub_drag_fraction(x as f32, screen_w as f32, screen_h as f32)
                        {
                            transport.seek_to_fraction(f64::from(fraction));
                        }
                    }
                    if let Some(state) = &drag {
                        if let Some(camera) = &mut camera {
                            let dx = f64::from(x - state.start_cursor.0);
                            camera.center_x = state.start_center_x - dx / camera.zoom;
                            camera.normalize(screen_h);
                        }
                    }
                }
                WindowEvent::MouseDown { button, x, y } => match button {
                    // Left click/drag on the scrub bar seeks the timeline
                    // (which rewrites only the state->color table below).
                    MouseButton::Left => {
                        if debug_overlay_enabled {
                            if let Some(fraction) = overlay::scrub_fraction_at(
                                x as f32,
                                y as f32,
                                screen_w as f32,
                                screen_h as f32,
                            ) {
                                transport.seek_to_fraction(f64::from(fraction));
                                scrub_drag = true;
                            }
                        }
                    }
                    MouseButton::Middle | MouseButton::Right => {
                        if let Some(camera) = &camera {
                            drag = Some(DragState {
                                start_cursor: (x, y),
                                start_center_x: camera.center_x,
                            });
                        }
                    }
                },
                WindowEvent::MouseUp { button } => {
                    if button == MouseButton::Left {
                        scrub_drag = false;
                    } else {
                        drag = None;
                    }
                }
                WindowEvent::Wheel { .. } => {
                    // Fail closed. C03 proves that wheel input accumulates on
                    // a 3D target eye along the normalized cursor ray, then
                    // flows through C02 interpolation and projected-footprint
                    // repair. Portable replay currently carries none of that
                    // current/target state, so a 2D zoom-factor substitute is
                    // not an admissible production behavior.
                    drag = None;
                }
                WindowEvent::CaptureLost => {
                    drag = None;
                    scrub_drag = false;
                }
            }
        }
        if closing {
            continue;
        }

        // Horizontal keyboard pan. Vertical input remains blocked until the
        // exact C03 projected-footprint repair can consume recorded 3D state.
        let pan_x = (f64::from(pan_keys.east as u8) - f64::from(pan_keys.west as u8))
            * KEY_PAN_SCREEN_PX_PER_SEC
            * dt;
        if pan_x != 0.0 {
            if let Some(camera) = &mut camera {
                camera.pan_screen(pan_x, f64::from(gfx.height));
            }
        }

        // E1 playback advance: steps recorded hours at the current speed and
        // auto-pauses at the last hour (product playback does not wrap).
        transport.advance(dt);
        let hour_index = transport.current_index();

        // Scrub: validate replay/controller color inputs only when their
        // signatures change. Visible political recoloring stays blocked until
        // the engine's primary fill and GradientBorder generation law is ported.
        let hour = transport.current_hour();
        let world_frame = provider.world_frame_at(hour)?;
        if clear_battleplan_selection {
            battleplan_selection.clear();
        } else if let Some(frame) = world_frame.as_deref() {
            if cycle_battleplan_owner {
                battleplan_selection.cycle_owner(frame);
            }
            if cycle_battleplan_order {
                battleplan_selection.cycle_order(frame);
            }
        } else if cycle_battleplan_owner || cycle_battleplan_order {
            // A requested selection cannot survive an unavailable frame: no
            // prior-hour identity may masquerade as current product input.
            battleplan_selection.clear();
        }
        if uploaded_hour_index != Some(hour_index) {
            if let Some(frame) = world_frame.as_deref() {
                transport.apply_recorded_session_timing(frame.session_metadata());
                let bindings = static_unit_catalogs.bind_frame(frame);
                let identity = format!(
                    "STATIC_UNIT_CATALOGS\0{}\0{}\0{}\0{}\0{}\0{}\0{:?}\0{:?}\0{:?}",
                    bindings.measured_template_names,
                    bindings.authored_template_matches,
                    bindings.measured_equipment_archetypes,
                    bindings.authored_equipment_matches,
                    bindings.measured_ship_definitions,
                    bindings.authored_ship_hull_matches,
                    bindings.unresolved_template_names,
                    bindings.unresolved_equipment_archetypes,
                    bindings.unresolved_ship_definitions,
                );
                if reported_production_data_needs.insert(identity) {
                    eprintln!(
                        "replay_app: packaged static unit catalogs consumed for hour {}: templates {}/{}, equipment archetypes {}/{}, ship definitions {}/{}; unresolved dynamic identities templates={:?} equipment={:?} ships={:?}",
                        frame.hour_index(),
                        bindings.authored_template_matches,
                        bindings.measured_template_names,
                        bindings.authored_equipment_matches,
                        bindings.measured_equipment_archetypes,
                        bindings.authored_ship_hull_matches,
                        bindings.measured_ship_definitions,
                        bindings.unresolved_template_names,
                        bindings.unresolved_equipment_archetypes,
                        bindings.unresolved_ship_definitions,
                    );
                }
                let research_bindings = static_research_catalogs
                    .bind_frame(frame)
                    .map_err(|error| {
                        format!(
                            "bind packaged static research catalogs at hour {}: {error}",
                            frame.hour_index()
                        )
                    })?;
                let research_identity = format!(
                    "STATIC_RESEARCH_CATALOGS\0{}\0{}\0{}\0{}\0{:?}\0{:?}\0{:?}",
                    research_bindings.focus_countries,
                    research_bindings.focus_labels,
                    research_bindings.research_countries,
                    research_bindings.technology_labels,
                    research_bindings.focus_unavailable,
                    research_bindings.research_unavailable,
                    research_bindings
                        .technology_sharing_definition_unavailable_ordinals,
                );
                if reported_production_data_needs.insert(research_identity) {
                    eprintln!(
                        "replay_app: packaged static research catalogs consumed for hour {}: focus labels {} across {} countries, technology labels {} across {} countries, technology-sharing groups {}; unavailable focus={:?} research={:?} sharing definition ordinals={:?}",
                        frame.hour_index(),
                        research_bindings.focus_labels,
                        research_bindings.focus_countries,
                        research_bindings.technology_labels,
                        research_bindings.research_countries,
                        research_bindings.technology_sharing_groups,
                        research_bindings.focus_unavailable,
                        research_bindings.research_unavailable,
                        research_bindings
                            .technology_sharing_definition_unavailable_ordinals,
                    );
                }
            }
            // The retained capture bundle is not a temporal fallback. Product
            // playback requires the current WorldFrame's authenticated render
            // carrier; absent/unavailable state must remain visually
            // unavailable rather than showing pixels from the bundle's
            // unrelated historical moment.
            production_map_refused_for_displayed_hour = world_frame
                .as_deref()
                .is_none_or(|frame| frame.production_render_status() != ViewStatus::Present);
            let owners = provider.owners_at(hour)?;
            let controllers = provider.controllers_at(hour)?;
            if let (Some(renderer), Some(bundle)) = (renderer.as_mut(), bundle.as_ref()) {
                renderer.update_state_politics(
                    &gfx,
                    bundle,
                    world_frame.as_deref(),
                    &owners,
                    &controllers,
                    &bundle.country_colors,
                )?;
            }
            // Validate capture-independent production authority on every
            // displayed hour, even while a named pixel carrier is omitted.
            // This consumes neither a debug capture nor a blank fallback.
            let production_plan = production_map.prepare_frame_with_rail(
                world_frame
                    .as_deref()
                    .and_then(replay_core::WorldFrame::production_render),
                world_frame
                    .as_deref()
                    .map(replay_core::WorldFrame::rail_replay),
                world_frame
                    .as_deref()
                    .map(replay_core::WorldFrame::production_by_country),
            )?;
            // Product playback validates the complete typed D3D carrier even
            // when no authenticated pixel route is executable. This is a
            // fail-closed schema/admission gate only: it never promotes an
            // empty plan, debug capture, or proof target into visible pixels.
            let production_d3d_plan = map_viewer::prepare_production_d3d_frame(
                &production_plan,
                production_map.resources(),
            )?;
            if production_d3d_plan.authenticated_ui_pass.is_none() {
                return Err(
                    "production replay frame has no complete authenticated selected-Present UI pass"
                        .into(),
                );
            }
            prepared_selected_present_ui = production_d3d_plan
                .authenticated_ui_pass
                .as_ref()
                .map(|_| production_d3d_plan.clone());
            for need in &production_plan.omitted_data_needs {
                let identity = format!("{}\0{}\0{}", need.packet, need.feature, need.reason);
                if reported_production_data_needs.insert(identity) {
                    eprintln!(
                        "replay_app: production non-UI omitted [{}] {}: {}",
                        need.packet, need.feature, need.reason
                    );
                }
            }
            for omission in &production_d3d_plan.omissions {
                let identity =
                    format!("PRODUCTION_MAP_D3D\0{}\0{}", omission.pass, omission.reason);
                if reported_production_data_needs.insert(identity) {
                    eprintln!(
                        "replay_app: production non-UI omitted [PRODUCTION_MAP_D3D] {}: {}",
                        omission.pass, omission.reason
                    );
                }
            }
            // The carried target, geometry, viewport, and output descriptors
            // are the complete authority for this frame. Do not impose an
            // unrelated semantic province-raster dependency: doing so would
            // suppress authenticated M03/O04/runtime draws that never consume
            // that optional content. Individual operations still fail closed
            // on every UsedContentKey they actually reference.
            prepared_production_d3d = Some(production_d3d_plan);
            uploaded_hour_index = Some(hour_index);
        }

        // CPU picking from the province-id mirror (never from pixels).
        let probe = match (cursor, renderer.as_ref(), camera.as_ref(), bundle.as_ref()) {
            (Some((x, y)), Some(renderer), Some(camera), Some(bundle)) => {
                let map_pos = renderer.screen_to_map(
                    camera,
                    f64::from(x) + 0.5,
                    f64::from(y) + 0.5,
                    f64::from(gfx.width),
                    f64::from(gfx.height),
                )?;
                probe_at(bundle, provider.as_ref(), hour, camera, map_pos)?
            }
            _ => String::from("map omitted"),
        };
        let playing = if transport.is_playing() {
            " [playing]"
        } else {
            ""
        };
        // A selected POV is display-admitted only when this exact WorldFrame
        // carries complete d94 visibility and d95 spotted-unit state.
        let observer_render_status = pov.render_status(world_frame.as_deref());
        let observer_render_admitted =
            !matches!(&observer_render_status, PovRenderStatus::Unavailable { .. });
        let observer_note = match &observer_render_status {
            PovRenderStatus::Omniscient => String::from("observer: world"),
            PovRenderStatus::Recorded {
                pov,
                province_slot_count,
                spotted_land_unit_count,
                spotted_taskforce_count,
                ..
            } => format!(
                "observer: {pov} (d94 provinces {province_slot_count}; d95 land {spotted_land_unit_count:?}, taskforces {spotted_taskforce_count:?})"
            ),
            PovRenderStatus::Unavailable { pov, diagnostic } => {
                format!("observer: {pov} ({diagnostic})")
            }
        };
        let speed = transport.speed_hours_per_sec();
        let hour_label = provider.hour_label(hour)?;
        let recorded_clock_suffix = transport
            .recorded_clock()
            .map(|clock| {
                let state = if clock.paused { "paused" } else { "running" };
                let tick = clock
                    .tick_progress_f32_bits
                    .map(f32::from_bits)
                    .map(|value| format!(" tick {value:.3}"))
                    .unwrap_or_default();
                format!(" | game speed {} {state}{tick}", clock.speed_raw + 1)
            })
            .unwrap_or_default();
        // F4: surface any mid-playback anomaly holds (marked HELD), so a held
        // last-good value never reads as live. Empty on clean recordings.
        let anomaly_note = provider.anomaly_note(hour)?;
        let anomaly_suffix = anomaly_note
            .as_deref()
            .map(|note| format!(" | {note}"))
            .unwrap_or_default();
        // The readout lives in the in-window overlay when the debug overlay is
        // active; the title carries the full readout while overlay text is
        // unavailable, and ALWAYS carries the F4 anomaly note — the overlay is
        // debug-only and its line-2 draw can be skipped, so the title is the
        // one surface where a held value can never silently read as live.
        if overlay.as_ref().is_some_and(Overlay::has_text) {
            window.set_title(&format!(
                "replay_app - {mode_label}{recorded_clock_suffix}{anomaly_suffix}"
            ));
        } else {
            let zoom = camera.as_ref().map(|camera| camera.zoom);
            window.set_title(&format!(
                "replay_app - {mode_label} | {hour_label}{playing} {speed:.0}h/s{recorded_clock_suffix} | {observer_note} | {probe}{}{anomaly_suffix}",
                zoom.map(|value| format!(" | zoom {value:.2}")).unwrap_or_default()
            ));
        }

        if gfx.width > 0 && gfx.height > 0 {
            let day_night_hour =
                day_night_enabled.then(|| playback_day_night_hour_from_timeline_hour(hour));
            let mut production_presented = false;
            let mut legacy_presented = false;
            let building_priority = map_viewer::building_presentation_priority(
                prepared_production_d3d.as_ref(),
                world_frame
                    .as_deref()
                    .map(replay_core::WorldFrame::d48_buildings_status)
                    .unwrap_or(replay_core::ViewStatus::DomainAbsent),
            );
            let exact_render_epoch_selected = world_frame
                .as_deref()
                .is_some_and(|frame| frame.production_render_status() == ViewStatus::Present);
            if production_recording && !exact_render_epoch_selected {
                return Err(format!(
                    "production replay hour {hour} has no authenticated production-render program"
                )
                .into());
            }
            let authenticated_production_selected = production_map_refused_for_displayed_hour
                || exact_render_epoch_selected
                || building_priority
                    == map_viewer::BuildingPresentationPriority::AuthenticatedD80Operations;
            if authenticated_production_selected && observer_render_admitted {
                if let Some(frame) = prepared_production_d3d.as_ref() {
                    match product_production_d3d.execute(
                        &gfx,
                        &game_resolver,
                        production_map.resources(),
                        frame,
                    ) {
                        Ok(outcome) if outcome.presented_to_app_target => {
                            production_presented = true;
                            let identity =
                                format!("PRODUCTION_MAP_D3D\0executed\0{}", outcome.receipt_sha256);
                            if reported_production_data_needs.insert(identity) {
                                eprintln!(
                                    "replay_app: production non-UI executed {} ordered operations to the app target; receipt {}",
                                    outcome.executed_operations, outcome.receipt_sha256
                                );
                            }
                        }
                        Ok(_) => {
                            return Err(
                                "authenticated production operations produced no carried SwapchainBackBuffer output"
                                    .into(),
                            )
                        }
                        Err(error) => {
                            // The headless product route propagates every
                            // authenticated production execution error. The
                            // interactive route must do the same: continuing
                            // transport over a cleared black target would make
                            // a failed replay look successfully playable.
                            return Err(error);
                        }
                    }
                } else {
                    return Err(
                        "authenticated production map was selected without a prepared D3D frame"
                            .into(),
                    );
                }
            } else if observer_render_admitted {
                if let (Some(renderer), Some(camera)) = (renderer.as_mut(), camera.as_ref()) {
                    // The legacy captured renderer remains available only when the
                    // displayed hour has no authenticated D80 map operation set.
                    // Once exact carried operations are selected, an omission or
                    // refusal cannot silently fall back to different pixels.
                    renderer.render(&gfx, camera, day_night_hour, None)?;
                    legacy_presented = true;
                }
            }
            if !legacy_presented && !production_presented {
                gfx.clear_target([0.0, 0.0, 0.0, 1.0])?;
            }
            let mut selected_present_executed = false;
            if let (true, Some(frame)) =
                (observer_render_admitted, prepared_selected_present_ui.as_ref())
            {
                let pass = frame
                    .authenticated_ui_pass
                    .as_ref()
                    .expect("prepared selected-Present frame lost its UI pass");
                if product_selected_present_ui
                    .as_ref()
                    .is_none_or(|runtime| !runtime.is_for_pass(pass))
                {
                    product_selected_present_ui = Some(
                        ProductSelectedPresentRuntime::new_with_decision_catalog_bytes(
                            &gfx,
                            &game_resolver,
                            &content_admission,
                            pass,
                            provider.decision_catalog().map(|catalog| catalog.bytes()),
                            provider
                                .static_artifact("gfx_icon_asset_manifest_static")
                                .map(|artifact| artifact.bytes()),
                            Arc::clone(&static_research_catalogs),
                            Arc::clone(&static_focus_definitions),
                            Arc::clone(&static_focus_catalog),
                        )?,
                    );
                }
                let current_world_frame = world_frame
                    .as_deref()
                    .ok_or("selected-Present UI has no current world frame")?;
                let recorded_ui =
                    crate::recorded_ui_owner::recorded_ui_owner(current_world_frame, pass)
                        .map_err(|error| {
                            format!(
                                "selected-Present recorder-owned UI identity is unavailable: {error}"
                            )
                        })?;
                let runtime_text = match current_world_frame.runtime_text()? {
                    replay_core::Leaf::Measured(catalog) => Some(catalog),
                    replay_core::Leaf::Absent | replay_core::Leaf::Unobtained => None,
                };
                match product_selected_present_ui
                    .as_mut()
                    .expect("selected-Present runtime was constructed")
                    .execute(
                        &gfx,
                        &game_resolver,
                        frame,
                        current_world_frame,
                        recorded_ui.country.as_deref(),
                        recorded_ui.runtime_text_pov_country_or_global.as_str(),
                        runtime_text.as_ref(),
                        battleplan_selection.supplemental_inputs(),
                        IntelligenceUiSupplementalInputs::default(),
                        OccupationUiSupplementalInputs::default(),
                    )? {
                    SelectedPresentUiOutcome::Executed {
                        operation_count,
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
                    } => {
                        selected_present_executed = true;
                        let identity = format!(
                            "SELECTED_PRESENT_UI\0executed\0{operation_count}\0{m02_draw_count}\0{decision_panel_item_count:?}\0{power_balance_item_count:?}\0{intelligence_panel_count:?}\0{intelligence_unavailable_reason:?}\0{agency_upgrade_count:?}\0{cipher_target_count:?}\0{trade_license_count:?}\0{xui15_surface_drawn}\0{xui15_unavailable_reason:?}\0{occupation_state_count:?}\0{occupation_unavailable_reason:?}\0{battleplan_order_count:?}\0{battleplan_selected_order_present:?}\0{typed_consumption:?}"
                        );
                        if reported_production_data_needs.insert(identity) {
                            eprintln!(
                                "replay_app: production selected-Present UI executed {operation_count} exact operations including {m02_draw_count} M02 draws; same-Present typed companions A34={decision_panel_item_count:?}, A37={power_balance_item_count:?}, XUI14={intelligence_panel_count:?}, XUI14_unavailable={intelligence_unavailable_reason:?}, A30_agency={agency_upgrade_count:?}, A30_cipher={cipher_target_count:?}, X3_licences={trade_license_count:?}, XUI15_drawn={xui15_surface_drawn}, XUI15_unavailable={xui15_unavailable_reason:?}, XUI16={occupation_state_count:?}, XUI16_unavailable={occupation_unavailable_reason:?}, d54_orders={battleplan_order_count:?}, d54_selected_order={battleplan_selected_order_present:?}, remaining={typed_consumption:?}"
                            );
                        }
                    }
                    SelectedPresentUiOutcome::Omitted(reason) => {
                        return Err(format!(
                            "production selected-Present UI could not execute its complete authenticated frontier: {reason}"
                        )
                        .into());
                    }
                }
            }
            // No authenticated selected-Present pass means no game HUD
            // pixels. Never substitute the legacy flattened compositor.
            // Overlay after the map: date/status text in the real game
            // font plus the scrub bar. Line 1 is the raw Clausewitz date
            // string from the recording (GAP: localisation-formatted dates
            // like the in-game clock are a later slice).
            let line2 = format!(
                "{mode_label}{playing} {speed:.0}h/s | {POLITICAL_COLOR_BLOCKER_BADGE} | {observer_note} | {probe}{anomaly_suffix}"
            );
            // The legacy overlay is a replay debug/control surface with proxy
            // geometry. It is legal only on the explicitly requested capture
            // path and is never a fallback for omitted production HUD pixels.
            if debug_overlay_enabled && !selected_present_executed {
                if let Some(overlay) = &mut overlay {
                    overlay.draw(&gfx, &hour_label, &line2, hour_index, transport.len())?;
                }
            }
            // Present never blocks (PresentOutcome): a full queue or a
            // fully-occluded window turns into a short sleep with the
            // message pump still alive, instead of freezing all input.
            match gfx.present()? {
                crate::d3d::PresentOutcome::Presented => {}
                crate::d3d::PresentOutcome::Skipped => {
                    std::thread::sleep(std::time::Duration::from_millis(4));
                }
                crate::d3d::PresentOutcome::Occluded => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
    Ok(())
}

fn probe_at(
    bundle: &MapBundle,
    provider: &dyn OwnerProvider,
    hour: i64,
    camera: &Camera,
    map_pos: Option<(f64, f64)>,
) -> Result<String> {
    // Captured-pass picking intersects the same camera ray with y=0. the game
    // terrain-height refinement is still a named gap; province lookup remains
    // from the CPU province-id mirror rather than pixels.
    let Some((map_x, map_y)) = map_pos else {
        return Ok(String::from("prov - state - owner - controller -"));
    };
    if map_y < 0.0 || map_y >= bundle.height as f64 {
        return Ok(String::from("prov - state - owner - controller -"));
    }
    let x = camera.wrap_x(map_x) as usize;
    let y = map_y as usize;
    if x >= bundle.width || y >= bundle.height {
        return Ok(String::from("prov - state - owner - controller -"));
    }
    let province = bundle.province_ids[y * bundle.width + x];
    let state = bundle.province_to_state.get(&province);
    let owners = provider.owners_at(hour)?;
    let controllers = provider.controllers_at(hour)?;
    let owner = state.and_then(|state_id| owners.get(state_id));
    let controller = state.and_then(|state_id| controllers.get(state_id));
    Ok(format!(
        "prov {} state {} owner {} controller {}",
        province,
        state.map_or_else(|| String::from("-"), u32::to_string),
        owner.map_or("-", String::as_str),
        controller.map_or("-", String::as_str)
    ))
}

#[cfg(test)]
mod product_production_d3d_tests {
    use super::*;
    use portable_content_manifest::{
        ContentLayerKind, ContentManifest, LayerStorageKind, PlaybackContentInventory,
        PlaybackContentLayer, RecordedContentLayer, RequiredContent, RequiredContentKind,
        ResolvedPlaybackContent, SourceGameIdentity, SourceProfileIdentity, UsedContentKey, SCHEMA,
    };
    use replay_core::{
        BlendDesc, BlendFactor, BlendOperation, BufferSource, Comparison, CompiledShaderPayload,
        CullMode, DepthDesc, DrawCommand, FillMode, GeometryBinding, IndexFormat, InputElementDesc,
        PipelineDesc, PreparedRenderTarget, PrimitiveTopology, RasterizerDesc, RenderExecutionRef,
        RenderOutputState, RenderTargetDesc, ShaderProgram, StaticGeometrySource, StaticMapCommand,
        StaticMapDraw, StaticMapOperation, StaticMapPassProgram, StencilFaceDesc, StencilOperation,
        VertexBufferBinding, ViewportDesc,
    };
    use sha2::{Digest, Sha256};
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::Arc;

    struct StartupDateProbe {
        hours: Vec<i64>,
        date_calls: Cell<usize>,
        frame_calls: Cell<usize>,
    }

    impl OwnerProvider for StartupDateProbe {
        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn owners_at(&self, _hour: i64) -> Result<Arc<BTreeMap<u32, String>>> {
            panic!("transport startup must not materialize owner maps")
        }

        fn controllers_at(&self, _hour: i64) -> Result<Arc<BTreeMap<u32, String>>> {
            panic!("transport startup must not materialize controller maps")
        }

        fn world_frame_at(&self, _hour: i64) -> Result<Option<Arc<WorldFrame>>> {
            self.frame_calls.set(self.frame_calls.get() + 1);
            Ok(None)
        }

        fn date_at(&self, hour: i64) -> Result<Option<String>> {
            self.date_calls.set(self.date_calls.get() + 1);
            Ok(Some(format!("1936.1.1.{hour}")))
        }
    }

    #[test]
    fn startup_transport_binding_does_not_enumerate_dates_or_load_frames() {
        let provider = StartupDateProbe {
            hours: (0..20).collect(),
            date_calls: Cell::new(0),
            frame_calls: Cell::new(0),
        };

        let transport = transport_from_provider_index(&provider).expect("transport index");

        assert_eq!(transport.len(), 20);
        assert_eq!(provider.date_calls.get(), 0);
        assert_eq!(provider.frame_calls.get(), 0);
    }

    #[test]
    fn battleplan_selection_is_fail_closed_until_product_input_selects_an_owner() {
        let selection = BattleplanSelectionState::default();
        let inputs = selection.supplemental_inputs();
        assert_eq!(inputs.selected_owner, None);
        assert_eq!(inputs.selected_order_key, None);
    }

    #[test]
    fn battleplan_selection_preserves_stable_order_identity_and_clears_cross_owner_state() {
        let army = ArmyBattleplanOwnerKey {
            kind: replay_core::ArmyBattleplanOwnerKind::Army,
            id: 17,
        };
        let selected = ArmyBattleplanOrderKey {
            owner: army,
            engine_order_instance_id: 203,
            generation: 9,
        };
        let mut selection = BattleplanSelectionState::default();
        selection.select_order(selected.clone());

        let inputs = selection.supplemental_inputs();
        assert_eq!(inputs.selected_owner, Some(army));
        assert_eq!(inputs.selected_order_key, Some(&selected));

        let army_group = ArmyBattleplanOwnerKey {
            kind: replay_core::ArmyBattleplanOwnerKind::ArmyGroup,
            id: 18,
        };
        selection.select_owner(army_group);
        let inputs = selection.supplemental_inputs();
        assert_eq!(inputs.selected_owner, Some(army_group));
        assert_eq!(
            inputs.selected_order_key, None,
            "an order identity must not leak into a different typed owner selection"
        );

        selection.clear();
        let inputs = selection.supplemental_inputs();
        assert_eq!(inputs.selected_owner, None);
        assert_eq!(inputs.selected_order_key, None);
    }

    fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn admitted_resolver_and_manifest(
        root: &std::path::Path,
        required: UsedContentKey,
        consumer: &str,
    ) -> (GameFileResolver, ContentManifest) {
        fs::create_dir_all(root).unwrap();
        let mut manifest = ContentManifest {
            schema: SCHEMA.into(),
            product_modset_sha256: "09".repeat(32),
            source_game: SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256:
                    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad".into(),
                build_profile: "release".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "product-route-test".into(),
                playset_id: "product-route-test".into(),
                selection_sha256: String::new(),
            },
            effective_vfs_manifest_sha256: "11".repeat(32),
            ordered_layers: vec![RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "the game".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            }],
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            required_content: vec![RequiredContent {
                content_key: "test-shader".into(),
                kind: RequiredContentKind::ShaderSource,
                content: required.clone(),
                observation_count: 1,
                consumers: vec![consumer.into()],
            }],
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        let manifest = manifest.with_computed_digest().unwrap();
        let inventory = PlaybackContentInventory {
            source_game: manifest.source_game.clone(),
            source_profile: manifest.source_profile.clone(),
            layers: vec![PlaybackContentLayer {
                recorded_ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
            }],
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            settings: BTreeMap::new(),
            resolved_content: vec![ResolvedPlaybackContent {
                normalized_game_relative_path: required.normalized_game_relative_path.clone(),
                winning_layer_ordinal: required.winning_layer_ordinal,
                layer_kind: required.layer_kind,
                layer_id: required.layer_id.clone(),
                source_sha256: required.source_sha256.clone(),
                payload_sha256: required.payload_sha256.clone(),
                payload_bytes: required.payload_bytes,
            }],
        };
        let admission = manifest.admit_playback(&inventory).unwrap();
        let resolver = GameFileResolver::vanilla(root)
            .unwrap()
            .with_content_admission(&admission)
            .unwrap();
        (resolver, manifest)
    }

    fn admitted_resolver(root: &std::path::Path, required: UsedContentKey) -> GameFileResolver {
        admitted_resolver_and_manifest(root, required, "test.product_route.executor").0
    }

    fn presentation_draw_frame(
        source: UsedContentKey,
        vertex_shader: Vec<u8>,
        pixel_shader: Vec<u8>,
    ) -> map_viewer::PreparedProductionD3dFrame {
        let mut vertices = Vec::new();
        for value in [-1.0f32, -1.0, -1.0, 3.0, 3.0, -1.0] {
            vertices.extend_from_slice(&value.to_le_bytes());
        }
        let stencil = StencilFaceDesc {
            fail_operation: StencilOperation::Keep,
            depth_fail_operation: StencilOperation::Keep,
            pass_operation: StencilOperation::Keep,
            comparison: Comparison::Always,
        };
        let draw = StaticMapDraw {
            draw_token: "test.product_route.solid_red".into(),
            shader_program: ShaderProgram {
                source,
                producer_registration_root_rva: None,
                effect_token: "test.solid_red".into(),
                vertex_entry_token: "vs_main".into(),
                pixel_entry_token: "ps_main".into(),
                ordered_defines: Vec::new(),
                compiled_vertex_shader: CompiledShaderPayload {
                    compiler_identity: "d3dcompiler_product_route_test".into(),
                    target_profile: "vs_5_0".into(),
                    compiler_flags_u64: 0,
                    sha256: sha256(&vertex_shader),
                    bytecode: vertex_shader,
                },
                compiled_pixel_shader: CompiledShaderPayload {
                    compiler_identity: "d3dcompiler_product_route_test".into(),
                    target_profile: "ps_5_0".into(),
                    compiler_flags_u64: 0,
                    sha256: sha256(&pixel_shader),
                    bytecode: pixel_shader,
                },
                geometry_shader: None,
                hull_shader: None,
                domain_shader: None,
                compute_shader: None,
            },
            ordered_shader_resources: Vec::new(),
            pipeline: PipelineDesc {
                topology: PrimitiveTopology::TriangleList,
                blend: BlendDesc {
                    alpha_to_coverage_enabled: false,
                    independent_blend_enabled: false,
                    enabled: false,
                    source_color: BlendFactor::One,
                    destination_color: BlendFactor::Zero,
                    color_operation: BlendOperation::Add,
                    source_alpha: BlendFactor::One,
                    destination_alpha: BlendFactor::Zero,
                    alpha_operation: BlendOperation::Add,
                    write_mask_rgba_u8: 0xf,
                    additional_targets: Vec::new(),
                    blend_factor_rgba_f32_bits: [0; 4],
                    sample_mask_u32: u32::MAX,
                },
                depth: DepthDesc {
                    enabled: false,
                    write_enabled: false,
                    comparison: Comparison::Always,
                    stencil_enabled: false,
                    stencil_read_mask_u8: 0xff,
                    stencil_write_mask_u8: 0xff,
                    front_face: stencil.clone(),
                    back_face: stencil,
                    stencil_reference_u32: 0,
                },
                rasterizer: RasterizerDesc {
                    fill: FillMode::Solid,
                    cull: CullMode::None,
                    front_counter_clockwise: false,
                    depth_clip_enabled: true,
                    scissor_enabled: false,
                    depth_bias_i32: 0,
                    depth_bias_clamp_f32_bits: 0,
                    slope_scaled_depth_bias_f32_bits: 0,
                    multisample_enabled: false,
                    antialiased_line_enabled: false,
                },
                samplers: Vec::new(),
                input_layout: vec![InputElementDesc {
                    semantic_token: "POSITION".into(),
                    semantic_index: 0,
                    dxgi_format_u32: 16,
                    input_slot: 0,
                    aligned_byte_offset: 0,
                    per_instance: false,
                    instance_step_rate: 0,
                }],
                index_format: IndexFormat::None,
                static_indices_u16: Vec::new(),
            },
            geometry: StaticGeometrySource::Exact {
                geometry: GeometryBinding {
                    ordered_vertex_buffers: vec![VertexBufferBinding {
                        input_slot: 0,
                        stride_bytes: 8,
                        byte_offset: 0,
                        source: BufferSource::Inline {
                            sha256: sha256(&vertices),
                            bytes: vertices,
                        },
                    }],
                    index_buffer: None,
                    stream_output_targets: Vec::new(),
                    draw: DrawCommand::Draw {
                        vertex_count: 3,
                        start_vertex: 0,
                    },
                },
            },
            ordered_constant_buffers: Vec::new(),
            output_state: RenderOutputState {
                ordered_color_targets: vec![RenderTargetDesc {
                    prepared_target: PreparedRenderTarget::SwapchainBackBuffer,
                    width: 4,
                    height: 4,
                    dxgi_format_u32: winapi::shared::dxgiformat::DXGI_FORMAT_B8G8R8A8_UNORM,
                    mip_level: 0,
                    sample_count: 1,
                    sample_quality: 0,
                    persistent_content: None,
                    exact_resource: None,
                }],
                depth_target: None,
                viewport: ViewportDesc {
                    top_left_x_f32_bits: 0,
                    top_left_y_f32_bits: 0,
                    width_f32_bits: 4f32.to_bits(),
                    height_f32_bits: 4f32.to_bits(),
                    min_depth_f32_bits: 0,
                    max_depth_f32_bits: 1f32.to_bits(),
                },
                scissor: None,
            },
            payload_sha256: "22".repeat(32),
        };
        let operation = StaticMapOperation {
            operation_ordinal: 0,
            payload_sha256: "33".repeat(32),
            command: StaticMapCommand::Draw { draw },
        };
        map_viewer::PreparedProductionD3dFrame {
            content_manifest_sha256: "33".repeat(32),
            product_modset_sha256: "99".repeat(32),
            effective_vfs_manifest_sha256: "11".repeat(32),
            c03_camera_replay: None,
            target: Some(map_viewer::ProductionMainSceneTargetDesc {
                color_attachment_ordinal: 4,
                color_width: 4,
                color_height: 4,
                color_format: 28,
                color_samples: 1,
                color_quality: 0,
                depth_attachment_ordinal: 9,
                depth_width: 4,
                depth_height: 4,
                depth_format: 45,
                depth_samples: 1,
                depth_quality: 0,
            }),
            ordered_scene_constant_buffers: Vec::new(),
            ordered_render_operations: vec![RenderExecutionRef::Static {
                operation_ordinal: 0,
            }],
            snow_mud_texture: None,
            political_textures: Vec::new(),
            tiled_lights: Vec::new(),
            ordered_static_used_content: Vec::new(),
            ordered_static_programs: vec![StaticMapPassProgram {
                program_ordinal: 0,
                pass_token: "diagnostic.product_route.solid_red".into(),
                ordered_content_keys: vec!["test-shader".into()],
                ordered_operations: vec![operation],
                payload_sha256: "44".repeat(32),
            }],
            authenticated_texture_snapshots: Vec::new(),
            rail_ribbon: None,
            mapname_d80_state: replay_core::MapnameD80State::Missing,
            tree_runtime: None,
            tree_materialized_execution_sha256: None,
            s1_shadow: None,
            ordered_runtime_draws: Vec::new(),
            m02_combat: None,
            authenticated_ui_pass: None,
            m03_arrows: None,
            rail_supply_arrows: None,
            omissions: Vec::new(),
        }
    }

    fn presentation_test_content() -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "11".repeat(32),
            normalized_game_relative_path: "mod/presentation_test.shader".into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "22".repeat(32),
            payload_sha256: "22".repeat(32),
            payload_bytes: 1,
        }
    }

    #[test]
    fn m03_only_swapchain_output_counts_as_app_presentation() {
        let mut frame = presentation_draw_frame(presentation_test_content(), vec![1], vec![2]);
        let StaticMapCommand::Draw { draw } =
            &frame.ordered_static_programs[0].ordered_operations[0].command
        else {
            panic!("fixture must contain a draw")
        };
        frame.m03_arrows = Some(replay_core::M03ArrowFrame {
            schema: "test".into(),
            executable_sha256: "11".repeat(32),
            static_semantic_proof_sha256: "22".repeat(32),
            update_index: 1,
            first_sequence: 1,
            last_sequence: 1,
            used_content: Vec::new(),
            rows: Vec::new(),
            draws: vec![replay_core::M03ArrowDraw {
                engine_draw_ordinal: 0,
                draw: draw.clone(),
                members: Vec::new(),
            }],
            ordered_draws: vec![replay_core::M03ArrowDrawRef {
                engine_draw_ordinal: 0,
            }],
        });
        frame.ordered_static_programs.clear();
        frame.ordered_render_operations.clear();
        assert!(frame_has_app_presentation(&frame));
        assert!(frame_has_authenticated_map_operations(&frame));
    }

    #[test]
    fn o04_only_swapchain_output_counts_as_app_presentation() {
        let source = presentation_test_content();
        let mut frame = presentation_draw_frame(source.clone(), vec![1], vec![2]);
        let StaticMapCommand::Draw { draw } = frame.ordered_static_programs[0].ordered_operations
            [0]
        .command
        .clone() else {
            panic!("fixture must contain a draw")
        };
        let output = replay_core::RenderOutputRef::Entity {
            pass_ordinal: 0,
            entity_instance_id: 7,
            draw_order: 0,
        };
        frame.ordered_runtime_draws = vec![map_viewer::PreparedRuntimeDraw::O04City {
            ordered_draw: output,
            submission: replay_core::EntitySubmission {
                pass_ordinal: 0,
                draw_order: 0,
                entity_instance_id: 7,
                resolved_entity_sha256: "33".repeat(32),
                content_identity: replay_core::EntityContentIdentity {
                    resolved_entity_asset: source.clone(),
                    mesh: source,
                    native_selection: replay_core::EntityNativeSelection {
                        entity_registration_token: "test".into(),
                        entity_group_index: 0,
                        mesh_draw_list_index: 0,
                        entity_variant_u8: 0,
                        mesh_draw_variant_u8: 0,
                        pass_selector_i32: 0,
                        alternate_list_u8: 0,
                        bind_renderer_target_u8: 0,
                    },
                    native_selection_sha256: "44".repeat(32),
                },
                shader_program: draw.shader_program,
                ordered_texture_bindings: Vec::new(),
                pipeline: draw.pipeline,
                geometry: match draw.geometry {
                    StaticGeometrySource::Exact { geometry } => geometry,
                    _ => unreachable!(),
                },
                ordered_constant_buffers: draw.ordered_constant_buffers,
                output_state: draw.output_state,
                world_transform_4x4_f32_bits: [
                    1f32.to_bits(),
                    0,
                    0,
                    0,
                    0,
                    1f32.to_bits(),
                    0,
                    0,
                    0,
                    0,
                    1f32.to_bits(),
                    0,
                    0,
                    0,
                    0,
                    1f32.to_bits(),
                ],
                state_flags: 0,
                skinning: replay_core::Skinning::NotSkinned,
            },
            generator_ordinal: 0,
        }];
        frame.ordered_static_programs.clear();
        frame.ordered_render_operations.clear();
        assert!(frame_has_app_presentation(&frame));
        assert!(frame_has_authenticated_map_operations(&frame));
    }

    #[test]
    fn authenticated_d80_operations_require_a_prepared_map_target() {
        let mut frame = presentation_draw_frame(presentation_test_content(), vec![1], vec![2]);
        assert!(frame_has_authenticated_map_operations(&frame));
        assert_eq!(
            map_viewer::building_presentation_priority(
                Some(&frame),
                replay_core::ViewStatus::Present
            ),
            map_viewer::BuildingPresentationPriority::AuthenticatedD80Operations,
            "authenticated D80 operations outrank typed semantic building state"
        );
        frame.target = None;
        assert!(!frame_has_authenticated_map_operations(&frame));
        assert_eq!(
            map_viewer::building_presentation_priority(
                Some(&frame),
                replay_core::ViewStatus::Present
            ),
            map_viewer::BuildingPresentationPriority::TypedSemanticStateOnly,
            "typed D48 state remains data-only without an executable D80 target"
        );
        assert_eq!(
            map_viewer::building_presentation_priority(
                Some(&frame),
                replay_core::ViewStatus::DomainUnavailable
            ),
            map_viewer::BuildingPresentationPriority::Unavailable
        );

        frame.target = Some(map_viewer::ProductionMainSceneTargetDesc {
            color_attachment_ordinal: 4,
            color_width: 4,
            color_height: 4,
            color_format: 28,
            color_samples: 1,
            color_quality: 0,
            depth_attachment_ordinal: 9,
            depth_width: 4,
            depth_height: 4,
            depth_format: 45,
            depth_samples: 1,
            depth_quality: 0,
        });
        frame.ordered_static_programs.clear();
        frame.ordered_runtime_draws.clear();
        frame.m03_arrows = None;
        assert!(!frame_has_authenticated_map_operations(&frame));
    }

    #[test]
    fn synthetic_admitted_frame_uses_normal_product_runtime_and_changes_app_target() {
        // This module only compiles on Windows. A production freeze must not
        // silently turn the sole app-target pixel proof into a skipped pass
        // when D3D11 device construction fails.
        let gfx = Gfx::create_offscreen(4, 4)
            .expect("production app-target proof requires a working D3D11 device");
        let root = std::env::temp_dir().join(format!(
            "replay_app_product_d3d_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        struct RemoveOnDrop(std::path::PathBuf);
        impl Drop for RemoveOnDrop {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = RemoveOnDrop(root.clone());
        let hlsl = r#"
struct VSIn { float2 position : POSITION; };
float4 vs_main(VSIn input) : SV_Position { return float4(input.position, 0.0, 1.0); }
float4 ps_main() : SV_Target { return float4(1.0, 0.0, 0.0, 1.0); }
"#;
        let shader_path = root.join("gfx").join("fx").join("product_route_test.hlsl");
        fs::create_dir_all(shader_path.parent().unwrap()).unwrap();
        fs::write(&shader_path, hlsl).unwrap();
        let unrestricted = GameFileResolver::vanilla(&root).unwrap();
        let actual = unrestricted
            .resolve_required_with_provenance(
                "gfx/fx/product_route_test.hlsl",
                "product route test shader",
            )
            .unwrap();
        let source = UsedContentKey {
            effective_vfs_manifest_sha256: "11".repeat(32),
            normalized_game_relative_path: actual.logical_path.to_ascii_lowercase(),
            winning_layer_ordinal: actual.source.ordinal as u32,
            layer_kind: ContentLayerKind::Base,
            layer_id: actual.source.source_id,
            source_sha256: actual.sha256.clone(),
            payload_sha256: actual.sha256,
            payload_bytes: actual.byte_len,
        };
        let resolver = admitted_resolver(&root, source.clone());
        let compile = |entry: &str, target: &str| -> Vec<u8> {
            unsafe {
                let blob = crate::render::compile_shader(hlsl, entry, target)
                    .expect("product route proof shader must compile");
                std::slice::from_raw_parts(
                    (*blob.as_raw()).GetBufferPointer() as *const u8,
                    (*blob.as_raw()).GetBufferSize(),
                )
                .to_vec()
            }
        };
        let frame = presentation_draw_frame(
            source,
            compile("vs_main", "vs_5_0"),
            compile("ps_main", "ps_5_0"),
        );
        gfx.clear_target([0.0, 0.0, 0.0, 1.0]).unwrap();
        let before = gfx.read_target().unwrap();
        assert!(before.chunks_exact(4).all(|pixel| pixel == [0, 0, 0, 255]));
        let mut runtime = ProductProductionD3dRuntime::default();
        let resources = std::collections::BTreeMap::new();
        let outcome = runtime
            .execute(&gfx, &resolver, &resources, &frame)
            .unwrap();
        assert_eq!(outcome.executed_operations, 1);
        assert_eq!(
            outcome.executed_draw_tokens,
            vec!["test.product_route.solid_red".to_string()]
        );
        assert!(outcome.presented_to_app_target);
        assert_eq!(outcome.receipt_sha256.len(), 64);
        let pixels = gfx.read_target().unwrap();
        assert_eq!(pixels.len(), 4 * 4 * 4);
        assert!(pixels
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 0, 255, 255]));
        assert_ne!(pixels, before);
        runtime.invalidate();
    }

    #[test]
    fn normal_product_route_executes_before_hud_and_never_uses_a_raw_copy() {
        // Scope every marker to PRODUCT code: slice off this test module so a
        // marker can never be satisfied by its own string literal below.
        let source = include_str!("app.rs");
        let source = &source[..source.find("#[cfg(test)]").expect("test module marker")];
        let execute = source
            .find("match product_production_d3d.execute(")
            .unwrap();
        let legacy = source
            .find("} else if observer_render_admitted")
            .unwrap();
        let selected_present = source
            .find("if let (true, Some(frame))")
            .unwrap();
        let hud = source.find("overlay.draw(&gfx").unwrap();
        assert!(execute < legacy);
        assert!(execute < selected_present);
        assert!(selected_present < hud);
        assert!(source.contains("if debug_overlay_enabled && !selected_present_executed"));
        assert!(source.contains("pov.render_status(world_frame.as_deref())"));
        assert!(source.contains("authenticated_production_selected && observer_render_admitted"));
        assert!(source.contains(
            "(observer_render_admitted, prepared_selected_present_ui.as_ref())"
        ));
        assert!(
            source.contains(
                "is_none_or(|frame| frame.production_render_status() != ViewStatus::Present)"
            ),
            "missing/unavailable per-hour render state must refuse the historical capture fallback"
        );
        assert!(
            source.contains("|| exact_render_epoch_selected"),
            "a present per-hour render carrier must select the exact production path even when its visible operation set is empty"
        );
        assert!(source.contains(
            "initial production replay frame has no authenticated production-render program"
        ));
        assert!(source.contains(
            "authenticated production operations produced no carried SwapchainBackBuffer output"
        ));
        assert!(source.contains(
            "authenticated production map was selected without a prepared D3D frame"
        ));
        assert!(
            !source.contains(
                "production non-UI omitted [PRODUCTION_MAP_D3D] native execution"
            ),
            "authenticated map execution errors must terminate playback, not become black-frame omissions"
        );
        let forbidden_dimension_consumer = ["production_map.province", "_raster"].concat();
        let forbidden_dimension_gate = ["PRODUCTION_MAP", "_DIMENSIONS"].concat();
        assert!(!source.contains(&forbidden_dimension_consumer));
        assert!(!source.contains(&forbidden_dimension_gate));
        let forbidden = ["copy_main_scene", "_to_proof_texture"].concat();
        assert!(!source.contains(&forbidden));
    }

    #[test]
    fn selected_present_callsite_uses_product_battleplan_selection() {
        let source = include_str!("app.rs");
        let source = &source[..source.find("#[cfg(test)]").expect("test module marker")];
        let selected_present = source
            .find("if let (true, Some(frame))")
            .expect("selected-Present product route");
        let selected_present = &source[selected_present..];

        assert!(selected_present.contains("battleplan_selection.supplemental_inputs()"));
        assert!(
            !selected_present.contains("ArmyBattleplanUiSupplementalInputs::default()"),
            "the product route must not discard the app's explicit army/order selection"
        );
    }
}
