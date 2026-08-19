
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use map_viewer::GameFileResolver;
use replay_core::{ArmyBattleplanOrderKey, ArmyBattleplanOwnerKey, Leaf, ViewStatus, WorldFrame};
use winapi::um::winuser::{
    VK_ADD, VK_ESCAPE, VK_LEFT, VK_NEXT, VK_OEM_4, VK_OEM_6, VK_OEM_COMMA, VK_OEM_MINUS,
    VK_OEM_PERIOD, VK_OEM_PLUS, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SPACE, VK_SUBTRACT, VK_TAB,
};

use crate::bundle::MapBundle;
use crate::camera::Camera;
use crate::d3d::Gfx;
use crate::furniture_render::FurnitureRenderPass;
use crate::observer::{povs_from_frame, PovPicker, PovRenderStatus, PovState};
use crate::overlay::{self, Overlay};
use crate::render::{
    playback_day_night_hour_from_timeline_hour, MapRenderer, POLITICAL_COLOR_BLOCKER_BADGE,
};
use crate::text::{color_to_rgba_bytes, QuadRenderer, Vertex};
use crate::timeline::{build_provider, OwnerProvider};
use crate::transport::Transport;
use crate::ui_runtime::{
    ArmyBattleplanUiSupplementalInputs, ObserverUiFrameDisposition,
    ObserverViewerActionDisposition, ProductObserverOccupationRuntime, ProductObserverUiRuntime,
    ProductionLinesUiSupplementalInputs,
};
use crate::win32::{MouseButton, Window, WindowEvent};
use crate::{Result, TimelineMode};

const LEGACY_DEBUG_KEY_PAN_SCREEN_PX_PER_SEC: f64 = 900.0;
const MAX_FRAME_DT: f64 = 0.25;
const WIN32_WHEEL_DELTA_UNITS: i32 = 120;

unsafe fn require_current_srv_device(
    gfx: &Gfx,
    resource: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
    label: &str,
) -> Result<()> {
    use winapi::um::unknwnbase::IUnknown;

    if resource.is_null() {
        return Err(format!("{label} is null").into());
    }
    let mut device = std::ptr::null_mut();
    (*resource).GetDevice(&mut device);
    if device.is_null() {
        return Err(format!("{label} returned no D3D11 device").into());
    }
    let is_current = device == gfx.device.as_raw();
    (*(device as *mut IUnknown)).Release();
    if !is_current {
        return Err(format!("{label} belongs to a different D3D11 device").into());
    }
    Ok(())
}

unsafe fn require_current_buffer_device(
    gfx: &Gfx,
    buffer: *mut winapi::um::d3d11::ID3D11Buffer,
    label: &str,
) -> Result<()> {
    use winapi::um::unknwnbase::IUnknown;

    if buffer.is_null() {
        return Err(format!("{label} is null").into());
    }
    let mut device = std::ptr::null_mut();
    (*buffer).GetDevice(&mut device);
    if device.is_null() {
        return Err(format!("{label} returned no D3D11 device").into());
    }
    let is_current = device == gfx.device.as_raw();
    (*(device as *mut IUnknown)).Release();
    if !is_current {
        return Err(format!("{label} belongs to a different D3D11 device").into());
    }
    Ok(())
}

const VK_A: i32 = 0x41;
const VK_B: i32 = 0x42;
const VK_C: i32 = 0x43;
const VK_D: i32 = 0x44;
const VK_E: i32 = 0x45;
const VK_F: i32 = 0x46;
const VK_G: i32 = 0x47;
const VK_H: i32 = 0x48;
const VK_I: i32 = 0x49;
const VK_J: i32 = 0x4A;
const VK_K: i32 = 0x4B;
const VK_L: i32 = 0x4C;
const VK_M: i32 = 0x4D;
const VK_N: i32 = 0x4E;
const VK_O: i32 = 0x4F;
const VK_P: i32 = 0x50;
const VK_Q: i32 = 0x51;
const VK_R: i32 = 0x52;
const VK_S: i32 = 0x53;
const VK_T: i32 = 0x54;
const VK_U: i32 = 0x55;
const VK_V: i32 = 0x56;
const VK_W: i32 = 0x57;
const VK_X: i32 = 0x58;
const VK_Y: i32 = 0x59;
const VK_1: i32 = 0x31;
const VK_9: i32 = 0x39;
const VK_F1: i32 = 0x70;
const VK_F11: i32 = 0x7a;

fn mapmode_uses_four_button_layout(frame: &WorldFrame) -> Option<bool> {
    match &frame.session_header().dlc_mask {
        Leaf::Measured(mask) => Some(mask & (1 << 5) != 0),
        Leaf::Absent | Leaf::Unobtained => None,
    }
}

fn authenticated_observer_mapmode_binding(
    selection: &map_viewer::ViewerMapModeSelection,
    use_four_button_layout: Option<bool>,
) -> game_ui::hud::MapmodeBinding {
    let selected_button = use_four_button_layout
        .and_then(|_| selection.visible_button())
        .filter(|button| (1..=11).contains(button))
        .unwrap_or_default();
    game_ui::hud::MapmodeBinding {
        selected_button,
        use_four_button_layout,
    }
}

fn apply_authenticated_viewer_disposition(
    disposition: &ObserverViewerActionDisposition,
    transport: &mut Transport,
    viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    viewer_map_mode: &mut map_viewer::ViewerMapModeSelection,
    map_mode_selection_epoch: &mut u64,
) -> Result<bool> {
    let mut map_selection_changed = false;
    match disposition {
        ObserverViewerActionDisposition::SetPlaybackSpeedStep(one_based_step) => {
            let zero_based_step = one_based_step
                .checked_sub(1)
                .ok_or("authenticated transport step is zero")?;
            transport.set_speed_step(usize::from(zero_based_step));
        }
        ObserverViewerActionDisposition::SelectMapmodeButton(ordinal) => {
            if viewer.mapmode.use_four_button_layout.is_none() {
                return Err(
                    "authenticated mapmode action has no exact active layout identity".into(),
                );
            }
            let mode = map_viewer::HardcodedMapMode::from_visible_button(*ordinal)
                .ok_or("authenticated mapmode button has no exact engine registry entry")?;
            let selected = map_viewer::ViewerMapModeSelection::Hardcoded(mode);
            if selected != *viewer_map_mode {
                *map_mode_selection_epoch = map_mode_selection_epoch
                    .checked_add(1)
                    .ok_or("viewer map-mode selection epoch overflow")?;
                *viewer_map_mode = selected;
                map_selection_changed = true;
            }
            viewer.mapmode.selected_button = *ordinal;
            viewer.theatre_settings_land_id = None;
            viewer.theatre_mode = match ordinal {
                1 => Some(game_ui::production_renderer::ObserverTheatreMode::Land),
                2 => Some(game_ui::production_renderer::ObserverTheatreMode::Navy),
                3 => Some(game_ui::production_renderer::ObserverTheatreMode::Air),
                _ => None,
            };
        }
        ObserverViewerActionDisposition::OpenFocusTree => {
            if viewer.selected_country.is_none() {
                return Err("authenticated focus-tree action has no selected country".into());
            }
            viewer.decision_panel_open = false;
            viewer.decision_scroll_px = 0;
            viewer.collapsed_decision_categories.clear();
            viewer.power_balance.close();
            viewer.focus.open();
        }
        ObserverViewerActionDisposition::OpenPowerBalance => {
            if viewer.selected_country.is_none() {
                return Err("authenticated power-balance action has no selected country".into());
            }
            viewer.decision_panel_open = false;
            viewer.decision_scroll_px = 0;
            viewer.collapsed_decision_categories.clear();
            viewer.politics.close();
            viewer.intelligence.close();
            viewer.focus.close();
            viewer.research.close();
            viewer.power_balance.open();
        }
        ObserverViewerActionDisposition::OpenLandTheatreSettings(object_id) => {
            if viewer.theatre_mode != Some(game_ui::production_renderer::ObserverTheatreMode::Land)
                || *object_id == 0
            {
                return Err("authenticated land settings action has no active land mode".into());
            }
            viewer.theatre_settings_land_id = Some(*object_id);
        }
        ObserverViewerActionDisposition::CloseLandTheatreSettings(object_id) => {
            if viewer.theatre_settings_land_id != Some(*object_id) {
                return Err("authenticated land settings close identity changed".into());
            }
            viewer.theatre_settings_land_id = None;
        }
        ObserverViewerActionDisposition::CloseProductionLines => {
            viewer.production_lines.panel_open = false;
        }
        ObserverViewerActionDisposition::ToggleProductionLineRow(identity) => {
            use game_ui::production_lines::ViewerRowPresentation;
            let next = match viewer.production_lines.row_presentation.get(identity) {
                None | Some(ViewerRowPresentation::Expanded) => ViewerRowPresentation::Collapsed,
                Some(ViewerRowPresentation::Collapsed) => ViewerRowPresentation::Expanded,
            };
            viewer
                .production_lines
                .set_row_presentation(identity.clone(), next);
        }
        ObserverViewerActionDisposition::Applied
        | ObserverViewerActionDisposition::RefusedGameplayMutation(_)
        | ObserverViewerActionDisposition::RefusedAbilityActivation { .. }
        | ObserverViewerActionDisposition::RefusedDecisionActivation { .. }
        | ObserverViewerActionDisposition::RefusedDecisionCategoryProviderActivation { .. }
        | ObserverViewerActionDisposition::RefusedEventOptionActivation { .. } => {}
    }
    Ok(map_selection_changed)
}

fn apply_selected_country_identity(
    viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    selected_country: Option<String>,
) -> bool {
    let country_changed = viewer.selected_country != selected_country;
    if country_changed {
        game_ui::character_ui::reconcile_character_country_selection(
            viewer.selected_country.as_deref(),
            selected_country.as_deref(),
            &mut viewer.character_selection,
        );
        viewer.theatre_settings_land_id = None;
        viewer.production_lines.reset_for_seek_or_country_change();
        viewer.abilities.reset_for_country_switch();
        viewer.focus.reset_for_seek_or_country_change();
        viewer.power_balance.reset_for_seek_or_country_change();
        viewer.research.reset_for_seek_or_country_change();
        viewer
            .politics
            .reconcile_country(selected_country.as_deref());
        viewer
            .intelligence
            .reconcile_country(selected_country.as_deref());
        viewer.trade = game_ui::trade_layout::CountryTradeViewerState::default();
        if let Some(designer) = viewer.designer.as_mut() {
            designer.anchor_design_id.clear();
            designer.anchor_catalogue_hour = -1;
            designer.anchor_catalogue_owner_tag = selected_country.clone().unwrap_or_default();
            designer.anchor_catalogue_family = designer.family;
            designer.selected_design_id = None;
            designer.excluded_country_filter_tags.clear();
            designer.scroll_wheel_steps = 0;
            designer.country_scroll_wheel_steps = 0;
        }
    }
    viewer.selected_country = selected_country;
    if viewer.selected_country.is_none() {
        viewer.trade_panel_open = false;
        viewer.production_lines.panel_open = false;
        viewer.designer = None;
        viewer.politics.close();
        viewer.intelligence.close();
        viewer.power_balance.close();
    }
    country_changed
}

fn apply_selected_country_identity_with_runtime_reset(
    runtime: &mut ProductObserverUiRuntime,
    viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    selected_country: Option<String>,
) -> bool {
    let changed = apply_selected_country_identity(viewer, selected_country);
    if changed {
        runtime.reset_country_dependent_ui_for_country_change(viewer);
    }
    changed
}

#[derive(Clone, Copy, Debug, Default)]
struct DesignerControlInput {
    toggle_open: bool,
    cycle_family: bool,
    cycle_selection: bool,
    toggle_our: bool,
    toggle_foreign: bool,
    toggle_obsolete: bool,
    toggle_country_filter: bool,
    scroll_delta: i32,
    country_scroll_delta: i32,
}

fn new_designer_viewer_state(
    country: &str,
    family: game_ui::readonly_design_peace::DesignerViewKind,
) -> game_ui::design_catalog::ObserverDesignerViewState {
    game_ui::design_catalog::ObserverDesignerViewState {
        family,
        anchor_design_id: String::new(),
        anchor_catalogue_hour: -1,
        anchor_catalogue_owner_tag: country.to_string(),
        anchor_catalogue_family: family,
        selected_design_id: None,
        our_filter_enabled: true,
        foreign_filter_enabled: true,
        obsolete_filter_enabled: false,
        excluded_country_filter_tags: Vec::new(),
        scroll_wheel_steps: 0,
        country_scroll_wheel_steps: 0,
    }
}

fn reconcile_designer_selection(
    state: &mut game_ui::design_catalog::ObserverDesignerViewState,
    ordered_design_ids: &[String],
) {
    if state.selected_design_id.as_ref().is_some_and(|selected| {
        !ordered_design_ids
            .iter()
            .any(|candidate| candidate == selected)
    }) {
        state.selected_design_id = None;
    }
}

fn apply_designer_controls(
    product_ui: &ProductObserverUiRuntime,
    frame: &WorldFrame,
    selected_country: Option<&str>,
    viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    input: DesignerControlInput,
) {
    if input.toggle_open {
        if viewer.designer.is_some() {
            viewer.designer = None;
        } else if let Some(country) = selected_country {
            viewer.designer = Some(new_designer_viewer_state(
                country,
                game_ui::readonly_design_peace::DesignerViewKind::Ship,
            ));
            viewer.trade_panel_open = false;
            viewer.production_lines.panel_open = false;
        }
    }
    let (Some(country), Some(state)) = (selected_country, viewer.designer.as_mut()) else {
        return;
    };
    if input.cycle_family {
        state.family = match state.family {
            game_ui::readonly_design_peace::DesignerViewKind::Ship => {
                game_ui::readonly_design_peace::DesignerViewKind::Tank
            }
            game_ui::readonly_design_peace::DesignerViewKind::Tank => {
                game_ui::readonly_design_peace::DesignerViewKind::Plane
            }
            _ => game_ui::readonly_design_peace::DesignerViewKind::Ship,
        };
        state.anchor_design_id.clear();
        state.anchor_catalogue_hour = -1;
        state.anchor_catalogue_owner_tag = country.to_string();
        state.anchor_catalogue_family = state.family;
        state.selected_design_id = None;
        state.excluded_country_filter_tags.clear();
        state.scroll_wheel_steps = 0;
        state.country_scroll_wheel_steps = 0;
    }
    if input.toggle_our {
        state.our_filter_enabled = !state.our_filter_enabled;
    }
    if input.toggle_foreign {
        state.foreign_filter_enabled = !state.foreign_filter_enabled;
        if !state.foreign_filter_enabled {
            state.excluded_country_filter_tags.clear();
        }
    }
    if input.toggle_obsolete {
        state.obsolete_filter_enabled = !state.obsolete_filter_enabled;
    }
    if input.scroll_delta < 0 {
        state.scroll_wheel_steps = state.scroll_wheel_steps.saturating_sub(1);
    } else if input.scroll_delta > 0 {
        state.scroll_wheel_steps = state.scroll_wheel_steps.saturating_add(1);
    }
    if input.country_scroll_delta < 0 {
        state.country_scroll_wheel_steps = state.country_scroll_wheel_steps.saturating_sub(1);
    } else if input.country_scroll_delta > 0 {
        state.country_scroll_wheel_steps = state.country_scroll_wheel_steps.saturating_add(1);
    }
    let mut membership_probe = state.clone();
    membership_probe.selected_design_id = None;
    let Ok(mut membership) =
        product_ui.designer_effective_membership(frame, country, &membership_probe)
    else {
        return;
    };
    if input.cycle_selection {
        state.selected_design_id = if membership.ordered_design_ids.is_empty() {
            None
        } else {
            let next = state
                .selected_design_id
                .as_ref()
                .and_then(|selected| {
                    membership
                        .ordered_design_ids
                        .iter()
                        .position(|candidate| candidate == selected)
                })
                .map_or(0, |index| (index + 1) % membership.ordered_design_ids.len());
            membership.ordered_design_ids.get(next).cloned()
        };
    } else {
        reconcile_designer_selection(state, &membership.ordered_design_ids);
    }
    if input.toggle_country_filter {
        if state.excluded_country_filter_tags.is_empty() {
            if let Some(tag) = membership.country_filter_tags.first() {
                state.excluded_country_filter_tags.push(tag.clone());
            }
        } else {
            state.excluded_country_filter_tags.clear();
        }
        membership_probe = state.clone();
        membership_probe.selected_design_id = None;
        if let Ok(filtered_membership) =
            product_ui.designer_effective_membership(frame, country, &membership_probe)
        {
            membership = filtered_membership;
            reconcile_designer_selection(state, &membership.ordered_design_ids);
        }
    }
}

fn apply_production_lines_scroll_offset(
    viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    offset_pixels: Option<i32>,
) {
    if let Some(offset_pixels) = offset_pixels {
        viewer.production_lines.scroll_offset_px = offset_pixels;
    }
}

#[derive(Clone, Debug, Default)]
struct PanKeys {
    west: bool,
    east: bool,
    north: bool,
    south: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct OccupationWheelAccumulator {
    territory: i64,
    activity: i64,
    equipment: i64,
}

fn route_occupation_wheel(
    viewer: &mut game_ui::occupation_compositor::OccupationViewerState,
    accumulator: &mut OccupationWheelAccumulator,
    region: Option<game_ui::occupation_compositor::OccupationInputRegion>,
    delta: i32,
    x: i32,
    y: i32,
) -> bool {
    use game_ui::occupation_compositor::OccupationPanel;
    let Some(region) =
        region.filter(|region| region.panel == viewer.panel && region.contains_scroll_point(x, y))
    else {
        return false;
    };
    debug_assert_eq!(region.panel, viewer.panel);
    let (partial, row) = match viewer.panel {
        OccupationPanel::Closed => return false,
        OccupationPanel::Territories => {
            (&mut accumulator.territory, &mut viewer.territory_scroll_row)
        }
        OccupationPanel::GarrisonActivities => {
            (&mut accumulator.activity, &mut viewer.activity_scroll_row)
        }
        OccupationPanel::GarrisonEquipment => {
            (&mut accumulator.equipment, &mut viewer.equipment_scroll_row)
        }
    };
    let accumulated = *partial + i64::from(delta);
    let wheel_delta = i64::from(WIN32_WHEEL_DELTA_UNITS);
    let notches = accumulated / wheel_delta;
    *partial = accumulated % wheel_delta;
    let magnitude = usize::try_from(notches.unsigned_abs()).unwrap_or(usize::MAX);
    if notches > 0 {
        *row = row.saturating_sub(magnitude);
    } else if notches < 0 {
        *row = row.saturating_add(magnitude);
    }
    true
}

fn occupation_controls_active(
    viewer: &game_ui::production_renderer::ObserverViewerUiState,
    selected_country: Option<&str>,
) -> bool {
    use game_ui::occupation_compositor::OccupationPanel;
    viewer.occupation.panel != OccupationPanel::Closed
        && selected_country.is_some_and(|country| {
            !country.trim().is_empty() && viewer.occupation.selected_country == country
        })
}

fn occupation_owns_pointer(
    region: Option<game_ui::occupation_compositor::OccupationInputRegion>,
    x: i32,
    y: i32,
) -> bool {
    region.is_some_and(|region| region.contains_panel_point(x, y))
}

fn character_owns_pointer(
    region: Option<game_ui::character_ui::CharacterInputRegion>,
    x: i32,
    y: i32,
) -> bool {
    region.is_some_and(|region| region.contains_panel_point(x, y))
}

fn route_character_wheel(
    viewer: &mut game_ui::character_ui::CharacterViewerSelection,
    partial: &mut i64,
    region: Option<game_ui::character_ui::CharacterInputRegion>,
    delta: i32,
    x: i32,
    y: i32,
) -> bool {
    if !matches!(
        &viewer.panel,
        game_ui::character_ui::CharacterPanel::ArmyRoster
            | game_ui::character_ui::CharacterPanel::NavyRoster
            | game_ui::character_ui::CharacterPanel::Aces
    ) || !region.is_some_and(|region| region.contains_scroll_point(x, y))
    {
        return false;
    }
    let accumulated = *partial + i64::from(delta);
    let wheel_delta = i64::from(WIN32_WHEEL_DELTA_UNITS);
    let notches = accumulated / wheel_delta;
    *partial = accumulated % wheel_delta;
    let magnitude = usize::try_from(notches.unsigned_abs()).unwrap_or(usize::MAX);
    if notches > 0 {
        viewer.scroll_row = viewer.scroll_row.saturating_sub(magnitude);
    } else if notches < 0 {
        viewer.scroll_row = viewer.scroll_row.saturating_add(magnitude);
    }
    true
}

fn apply_character_scroll_feedback(
    viewer: &mut game_ui::character_ui::CharacterViewerSelection,
    applied_row: Option<usize>,
) {
    if let Some(applied_row) = applied_row {
        viewer.scroll_row = applied_row;
    }
}

fn reset_xui17_for_seek(
    viewer: &mut game_ui::production_renderer::ObserverViewerUiState,
    region: &mut Option<game_ui::character_ui::CharacterInputRegion>,
    hit_targets: &mut Vec<game_ui::character_ui::CharacterHitTarget>,
    semantic_state: &mut Option<game_ui::character_ui::ObserverCharacterUi>,
) {
    viewer.character_selection = game_ui::character_ui::CharacterViewerSelection::default();
    *region = None;
    hit_targets.clear();
    *semantic_state = None;
}

fn apply_occupation_scroll_feedback(
    viewer: &mut game_ui::occupation_compositor::OccupationViewerState,
    applied_row: Option<usize>,
) {
    use game_ui::occupation_compositor::OccupationPanel;
    let Some(applied_row) = applied_row else {
        return;
    };
    match viewer.panel {
        OccupationPanel::Closed => {}
        OccupationPanel::Territories => viewer.territory_scroll_row = applied_row,
        OccupationPanel::GarrisonActivities => viewer.activity_scroll_row = applied_row,
        OccupationPanel::GarrisonEquipment => viewer.equipment_scroll_row = applied_row,
    }
}

#[derive(Clone, Debug)]
struct DragState {
    start_cursor: (i32, i32),
    start_camera: Camera,
}

fn accumulate_engine_wheel_notches(remainder_units: &mut i32, delta_units: i32) -> Result<i32> {
    if delta_units == 0 {
        return Ok(0);
    }
    if *remainder_units != 0 && remainder_units.signum() != delta_units.signum() {
        *remainder_units = 0;
    }
    let accumulated = remainder_units
        .checked_add(delta_units)
        .ok_or("Win32 wheel accumulator overflow")?;
    let emitted_notches = accumulated / WIN32_WHEEL_DELTA_UNITS;
    *remainder_units = accumulated % WIN32_WHEEL_DELTA_UNITS;
    Ok(emitted_notches)
}

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

    fn xmap15_selection(&self, frame: &WorldFrame) -> map_viewer::Xmap15ViewerSelection {
        map_viewer::xmap15_viewer_selection(frame, self.selected_order_key.as_ref())
    }
}

#[derive(Default)]
pub(crate) struct ProductProductionD3dRuntime {
    backend: Option<map_viewer::NativeProductionD3dBackend>,
}

#[derive(Default)]
struct ProductTerrainHandoffRuntime {
    handoff: Option<map_viewer::NativeXmap04TerrainHandoff>,
}

#[derive(Default)]
struct ProductPoliticalHandoffRuntime {
    handoff: Option<map_viewer::NativeXmap05PoliticalHandoff>,
}

#[derive(Default)]
struct ProductLightingHandoffRuntime {
    handoff: Option<map_viewer::NativeXmap09LightingHandoff>,
    shadow_plan: Option<map_viewer::ShadowScreenChainPlan>,
}

struct ProductWaterHandoffRuntime {
    handoff: Option<map_viewer::NativeXmap07WaterHandoff>,
    execution: ProductWaterExecutionStatus,
}

struct ProductRiversStraitsHandoffRuntime {
    handoff: Option<map_viewer::NativeXmap08RiversStraitsHandoff>,
    execution: ProductRiversStraitsExecutionStatus,
}

#[derive(Default)]
struct ProductThinBorderHandoffRuntime {
    handoff: Option<map_viewer::NativeXmap06ThinBorderHandoff>,
    early_draw: Option<map_viewer::Xmap06PhaseDrawReceipt>,
    late_draw: Option<map_viewer::Xmap06PhaseDrawReceipt>,
}

#[derive(Clone, Debug)]
enum ProductRiversStraitsExecutionStatus {
    Unavailable { reason: String },
    Present(map_viewer::Xmap08D3dHandoffReceipt),
}

impl Default for ProductRiversStraitsHandoffRuntime {
    fn default() -> Self {
        Self {
            handoff: None,
            execution: ProductRiversStraitsExecutionStatus::Unavailable {
                reason: "current XMAP08 prepared frame and compositor providers have not joined"
                    .into(),
            },
        }
    }
}

#[derive(Default)]
struct ProductXmap10HandoffRuntime {
    main_scene: Option<map_viewer::NativeProductionMainSceneTarget>,
    main_scene_generation: u64,
    pre_particle_scene: Option<map_viewer::Xmap10PreParticleSceneReceipt>,
    scene_cb0s: Vec<map_viewer::NativeXmap10SceneConstantBuffer>,
    screen_shadow: Option<map_viewer::NativeXmap10ScreenShadowHandoff>,
    screen_shadow_plan: Option<map_viewer::ShadowScreenChainPlan>,
    screen_shadow_terrain_program_sha256: Option<String>,
    terrain_draw: Option<map_viewer::Xmap04TerrainDrawReceipt>,
    map_label_renderer: Option<crate::map_labels::MapLabelRenderer>,
    map_label_program_sha256: Option<String>,
    map_label_alpha: Option<map_viewer::MapNameAlphaState>,
    map_label_draw: Option<map_viewer::Xmap17DrawReceipt>,
    xmap20_effects: Option<ProductXmap20ExecutionStatus>,
    xmap21_packet: Option<map_viewer::PreparedMapFurniturePacket>,
    xmap21_execution: Option<crate::furniture_render::MapFurnitureExecutionReceipt>,
    grading_table: Option<map_viewer::grading_law::PostEffectVolumeTable>,
    grading_defines: Option<map_viewer::grading_law::GraphicsGradingDefines>,
    region_weights: Option<map_viewer::PostEffectRegionWeightManifest>,
    postfx_schedule: Option<map_viewer::AssignedStageSchedule>,
    postfx: Option<map_viewer::NativeXmap10PostFxExecutor>,
    temporal_receipt: Option<map_viewer::Xmap10TemporalAdaptationReceiptRef>,
    temporal_hour_index: Option<i64>,
    completed_postfx: Option<map_viewer::Xmap10CompletedPostFxFrame>,
}

#[derive(Clone, Debug)]
enum ProductXmap20ExecutionStatus {
    AuthenticatedEmpty {
        prepared_frame_sha256: String,
        complete_enumeration_sha256: String,
        main_scene_resource_identity_sha256: String,
        main_scene_allocation_generation: u64,
        pre_particle_scene_receipt_sha256: String,
        scene_receipt_sha256: String,
    },
    Present(crate::xmap20_effects::Xmap20ExecutionReceipt),
}

impl ProductXmap20ExecutionStatus {
    fn scene_receipt_sha256(&self) -> &str {
        match self {
            Self::AuthenticatedEmpty {
                scene_receipt_sha256,
                ..
            } => scene_receipt_sha256,
            Self::Present(receipt) => &receipt.packet_draw_receipt_sha256,
        }
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::AuthenticatedEmpty {
                prepared_frame_sha256,
                complete_enumeration_sha256,
                main_scene_resource_identity_sha256,
                main_scene_allocation_generation,
                pre_particle_scene_receipt_sha256,
                scene_receipt_sha256,
            } => {
                for value in [
                    prepared_frame_sha256,
                    complete_enumeration_sha256,
                    main_scene_resource_identity_sha256,
                    pre_particle_scene_receipt_sha256,
                    scene_receipt_sha256,
                ] {
                    if value.len() != 64
                        || !value
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    {
                        return Err(
                            "XMAP20 authenticated-empty receipt has an invalid digest".into()
                        );
                    }
                }
                if *main_scene_allocation_generation == 0 {
                    return Err(
                        "XMAP20 authenticated-empty receipt has zero MainScene generation".into(),
                    );
                }
                let expected = snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(
                    "hoi4.xmap20.authenticated-empty-scene.v1",
                    prepared_frame_sha256,
                    complete_enumeration_sha256,
                    main_scene_resource_identity_sha256,
                    main_scene_allocation_generation,
                    pre_particle_scene_receipt_sha256,
                ))?);
                if scene_receipt_sha256 != &expected {
                    return Err("XMAP20 authenticated-empty scene receipt mismatch".into());
                }
                Ok(())
            }
            Self::Present(receipt) => receipt.validate(),
        }
    }
}

fn xmap10_temporal_hour_is_consecutive(prior: Option<i64>, current: i64) -> bool {
    prior.is_some_and(|prior| current == prior || current == prior.saturating_add(1))
}

pub(crate) struct ProductXmap10CurrentProviderBundle<'a> {
    pub(crate) main_scene: &'a map_viewer::NativeProductionMainSceneTarget,
    pub(crate) main_scene_cb0: map_viewer::NativeXmap10MapCopySceneCb0View<'a>,
    pub(crate) shadow_map: map_viewer::NativeXmap10ShaderResourceView<'a>,
    pub(crate) shadow_receipt: &'a map_viewer::Xmap10ScreenShadowExecutionReceipt,
}

#[derive(Debug)]
enum ProductWaterExecutionStatus {
    Unavailable {
        missing_provider: &'static str,
    },
    Present {
        underwater: map_viewer::Xmap04UnderwaterDrawReceipt,
        surface_draws: Vec<map_viewer::Xmap07SurfaceDrawReceipt>,
    },
}

impl Default for ProductWaterHandoffRuntime {
    fn default() -> Self {
        Self {
            handoff: None,
            execution: ProductWaterExecutionStatus::Unavailable {
                missing_provider:
                    "XMAP10 aggregate SceneCB0/MainScene/ShadowMap providers are not retained",
            },
        }
    }
}

struct ProductXmap12HandoffRuntime {
    owned: Option<map_viewer::NativeXmap12ResourceOwner>,
    execution: ProductXmap12ExecutionStatus,
}

struct ProductXmap13UnitRuntime {
    owned: Option<map_viewer::NativeXmap12ResourceOwner>,
    static_content: Option<[replay_core::UsedContentKey; 3]>,
    shadow_bridge: Option<map_viewer::PreparedXmap12RenderFrame>,
    execution: ProductXmap13UnitExecutionStatus,
}

#[derive(Debug)]
enum ProductXmap13UnitExecutionStatus {
    Unavailable { reason: String },
    Complete(map_viewer::Xmap13NativeComponentReceipt),
}

impl Default for ProductXmap13UnitRuntime {
    fn default() -> Self {
        Self {
            owned: None,
            static_content: None,
            shadow_bridge: None,
            execution: ProductXmap13UnitExecutionStatus::Unavailable {
                reason: "current XMAP13 unit frame/native providers have not joined".into(),
            },
        }
    }
}

#[derive(Debug)]
enum ProductXmap12ExecutionStatus {
    Unavailable { reason: String },
    AuthenticatedEmpty(map_viewer::Xmap12NativeExecutionReceipt),
    Present(map_viewer::Xmap12NativeExecutionReceipt),
}

impl Default for ProductXmap12HandoffRuntime {
    fn default() -> Self {
        Self {
            owned: None,
            execution: ProductXmap12ExecutionStatus::Unavailable {
                reason: "current XMAP12 frame/native providers have not joined".into(),
            },
        }
    }
}

struct ProductXmap12MainSceneBindings<'a> {
    native: &'a map_viewer::NativeXmap12MainSceneBindings,
}

struct ProductXmap12LightDepthBindings<'a> {
    enabled_partition_mask: u8,
    owned: &'a map_viewer::NativeXmap12ResourceOwner,
    frame: &'a map_viewer::PreparedXmap12RenderFrame,
    programs: &'a [map_viewer::NativeXmap12ShadowProgramBinding],
    partitions: &'a [map_viewer::NativeXmap12ShadowPartitionBinding],
}

#[derive(Default)]
struct ProductTreeHandoffRuntime {
    owner: Option<map_viewer::NativeXmap11ResourceOwner>,
    execution: Option<map_viewer::TreeNativeExecutionReceipt>,
}

struct ProductXmap11LightDepthBindings<'a> {
    owner: &'a map_viewer::NativeXmap11ResourceOwner,
    frame: &'a map_viewer::PreparedTreeFrame,
    partitions: &'a [map_viewer::NativeXmap12ShadowPartitionBinding],
}

enum ProductXmap12SharedProviderJoin {
    Ready(map_viewer::NativeXmap12SharedResourceBinding),
    Unavailable(String),
}

fn xmap12_checked_shared_provider(
    required: &map_viewer::NativeXmap12ReflectedSharedResource,
    mut binding: map_viewer::NativeXmap12SharedResourceBinding,
) -> ProductXmap12SharedProviderJoin {
    let actual_dimension =
        match unsafe { map_viewer::xmap12_native_srv_view_dimension(binding.view) } {
            Ok(dimension) => dimension,
            Err(error) => {
                return ProductXmap12SharedProviderJoin::Unavailable(format!(
                    "XMAP12 cannot inspect shared {} at t{}: {error}",
                    required.semantic, required.shader_register
                ))
            }
        };
    if actual_dimension != required.view_dimension {
        return ProductXmap12SharedProviderJoin::Unavailable(format!(
            "XMAP12 shared {} at t{} has SRV dimension {}, reflected shader requires {}",
            required.semantic, required.shader_register, actual_dimension, required.view_dimension
        ));
    }
    binding.view_dimension = actual_dimension;
    ProductXmap12SharedProviderJoin::Ready(binding)
}

fn xmap12_shared_provider(
    required: &map_viewer::NativeXmap12ReflectedSharedResource,
    xmap12: &map_viewer::NativeXmap12ResourceOwner,
    prepared: &map_viewer::PreparedFoundationFrame,
    terrain: &ProductTerrainHandoffRuntime,
    political: &ProductPoliticalHandoffRuntime,
    lighting: &ProductLightingHandoffRuntime,
    water: &ProductWaterHandoffRuntime,
    screen_shadow: &map_viewer::NativeXmap10ScreenShadowHandoff,
) -> ProductXmap12SharedProviderJoin {
    if let Some(binding) = xmap12.static_shared_resource(required) {
        return xmap12_checked_shared_provider(required, binding);
    }
    let binding = match required.semantic.as_str() {
        "LightIndexMap" | "LightDataMap" => {
            let Some(handoff) = lighting.handoff.as_ref() else {
                return ProductXmap12SharedProviderJoin::Unavailable(
                    "XMAP12 shared light resource has no retained XMAP09 owner".into(),
                );
            };
            let view = if required.semantic == "LightIndexMap" {
                handoff.light_index_map_view()
            } else {
                handoff.light_data_map_view()
            };
            map_viewer::NativeXmap12SharedResourceBinding {
                semantic: required.semantic.clone(),
                shader_register: required.shader_register,
                payload_sha256: view.binding.resource_sha256.clone(),
                descriptor_sha256: view.binding.descriptor_sha256.clone(),
                view_dimension: required.view_dimension,
                view: view.as_raw(),
            }
        }
        "SnowMudData" | "SnowMudTexture" => {
            let Some(handoff) = water.handoff.as_ref() else {
                return ProductXmap12SharedProviderJoin::Unavailable(
                    "XMAP12 shared SnowMud resource has no retained XMAP07 owner".into(),
                );
            };
            let view = match handoff.resource_view("SnowMudTexture") {
                Ok(view) => view.at_shader_register(required.shader_register),
                Err(error) => {
                    return ProductXmap12SharedProviderJoin::Unavailable(format!(
                        "XMAP12 retained XMAP07 owner cannot expose SnowMudTexture: {error}"
                    ))
                }
            };
            map_viewer::NativeXmap12SharedResourceBinding {
                semantic: required.semantic.clone(),
                shader_register: required.shader_register,
                payload_sha256: view.binding.resource_sha256.clone(),
                descriptor_sha256: view.binding.descriptor_sha256.clone(),
                view_dimension: required.view_dimension,
                view: view.as_raw(),
            }
        }
        "ShadowMap" => {
            let view = match screen_shadow.shadow_map_view() {
                Ok(view) => view.at_shader_register(required.shader_register),
                Err(error) => {
                    return ProductXmap12SharedProviderJoin::Unavailable(format!(
                        "XMAP12 retained XMAP10 owner cannot expose ShadowMap: {error}"
                    ))
                }
            };
            map_viewer::NativeXmap12SharedResourceBinding {
                semantic: required.semantic.clone(),
                shader_register: required.shader_register,
                payload_sha256: view.resource_sha256().to_owned(),
                descriptor_sha256: view.descriptor_sha256().to_owned(),
                view_dimension: required.view_dimension,
                view: view.as_raw(),
            }
        }
        semantic => {
            if let Some(handoff) = political.handoff.as_ref() {
                if let Ok(view) = handoff.shader_resource(semantic) {
                    return xmap12_checked_shared_provider(
                        required,
                        map_viewer::NativeXmap12SharedResourceBinding {
                            semantic: required.semantic.clone(),
                            shader_register: required.shader_register,
                            payload_sha256: view.resource_sha256.to_owned(),
                            descriptor_sha256: view.descriptor_sha256.to_owned(),
                            view_dimension: required.view_dimension,
                            view: view.shader_resource_view,
                        },
                    );
                }
            }
            let Some(terrain) = terrain.handoff.as_ref() else {
                return ProductXmap12SharedProviderJoin::Unavailable(
                    "XMAP12 shared static resource has no retained XMAP04 owner".into(),
                );
            };
            let view = terrain
                .terrain_static_resource(&prepared.terrain, semantic, required.shader_register)
                .or_else(|_| {
                    terrain.underwater_static_resource(
                        &prepared.terrain,
                        semantic,
                        required.shader_register,
                    )
                });
            let Ok(view) = view else {
                return ProductXmap12SharedProviderJoin::Unavailable(format!(
                    "effective XMAP12 shader requires {semantic} at t{}, but no current exact product owner exposes it",
                    required.shader_register
                ));
            };
            map_viewer::NativeXmap12SharedResourceBinding {
                semantic: required.semantic.clone(),
                shader_register: required.shader_register,
                payload_sha256: view.resource_sha256().to_owned(),
                descriptor_sha256: view.descriptor_sha256().to_owned(),
                view_dimension: required.view_dimension,
                view: view.as_raw(),
            }
        }
    };
    xmap12_checked_shared_provider(required, binding)
}

struct ProductRailHandoffRuntime {
    rail: Option<map_viewer::NativeXmap16RailHandoff>,
    supply: Option<map_viewer::NativeXmap16SupplyArrowHandoff>,
    hub: Option<map_viewer::NativeXmap16HubHandoff>,
    hub_packet: Option<map_viewer::PreparedXmap16HubGuiPacket>,
    hub_renderer: Option<QuadRenderer>,
    unavailable_reason: Option<String>,
    core_execution_complete: bool,
    rail_execution_sha256: Option<String>,
    supply_execution_sha256s: Vec<String>,
}

impl Default for ProductRailHandoffRuntime {
    fn default() -> Self {
        Self {
            rail: None,
            supply: None,
            hub: None,
            hub_packet: None,
            hub_renderer: None,
            unavailable_reason: Some("XMAP16 rail/supply foundation has not been retained".into()),
            core_execution_complete: false,
            rail_execution_sha256: None,
            supply_execution_sha256s: Vec::new(),
        }
    }
}

struct ProductXmap18HubMode<'a> {
    map_modes: &'a map_viewer::PreparedMapModeFrame,
}

impl map_viewer::Xmap16HubPresentationModeProvider for ProductXmap18HubMode<'_> {
    fn presentation_mode(
        &self,
        _viewer: &map_viewer::ViewerOwnedMapState,
    ) -> Result<Option<map_viewer::Xmap16HubPresentationMode>> {
        Ok(Some(if self.map_modes.xmap16_supply_view_active()? {
            map_viewer::Xmap16HubPresentationMode::Supply
        } else {
            map_viewer::Xmap16HubPresentationMode::Normal
        }))
    }
}

struct ProductXmap16HubGuiExecutor<'a> {
    gfx: &'a Gfx,
    renderer: &'a QuadRenderer,
    identity_sha256: String,
}

impl map_viewer::NativeXmap16HubGuiExecutor for ProductXmap16HubGuiExecutor<'_> {
    fn executor_identity_sha256(&self) -> &str {
        &self.identity_sha256
    }

    unsafe fn draw_xmap16_hub_quads(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        packet: &map_viewer::PreparedXmap16HubGuiPacket,
        textures: &map_viewer::NativeXmap16HubTextureSet,
    ) -> Result<()> {
        if context != self.gfx.context.as_raw() {
            return Err("XMAP16 hub executor received a foreign D3D11 context".into());
        }
        packet.validate()?;
        let white = color_to_rgba_bytes(0xffff_ffff);
        for quad in &packet.quads {
            let [top_left, top_right, bottom_left, bottom_right] = quad.vertices;
            let vertex = |source: map_viewer::Xmap16HubGuiVertex| Vertex {
                pos: source.position_px,
                uv: source.uv,
                color: white,
            };
            let vertices = [
                vertex(top_left),
                vertex(top_right),
                vertex(bottom_left),
                vertex(top_right),
                vertex(bottom_right),
                vertex(bottom_left),
            ];
            let srv = textures.shader_resource_view(quad.texture_role)?;
            self.renderer
                .draw_textured_vertices(self.gfx, srv, &vertices)?;
        }
        Ok(())
    }
}

impl ProductTerrainHandoffRuntime {
    fn retain_frame(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        prepared: &map_viewer::PreparedFoundationFrame,
    ) -> Result<&map_viewer::Xmap04D3dHandoffReceipt> {
        if self.handoff.as_ref().is_some_and(|handoff| {
            handoff.receipt().terrain_program_sha256
                != prepared.terrain.program.deterministic_sha256
        }) {
            self.handoff = None;
        }
        if self.handoff.is_none() {
            self.handoff = Some(unsafe {
                map_viewer::NativeXmap04TerrainHandoff::create_on_device(
                    gfx.device.as_raw(),
                    resolver,
                    &prepared.terrain,
                )?
            });
        } else {
            unsafe {
                self.handoff
                    .as_mut()
                    .ok_or("XMAP04 terrain handoff disappeared")?
                    .update_frame(gfx.device.as_raw(), &prepared.terrain)?;
            }
        }
        Ok(self
            .handoff
            .as_ref()
            .ok_or("XMAP04 terrain handoff was not retained")?
            .receipt())
    }

    fn xmap16_supply_resources(
        &self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
    ) -> Result<Vec<map_viewer::NativeXmap16ExternalResourceView>> {
        let handoff = self
            .handoff
            .as_ref()
            .ok_or("XMAP04 resources are absent at XMAP16 supply execution")?;
        handoff
            .map_shader_resource_views()?
            .into_iter()
            .map(|resource| {
                resource.validate_for_frame(
                    gfx.device.as_raw(),
                    &prepared.terrain.replay_frame_identity_sha256,
                    &prepared.terrain.camera_frame_sha256,
                )?;
                unsafe {
                    map_viewer::NativeXmap16ExternalResourceView::from_provider(
                        map_viewer::Xmap16SupplyArrowResourceIdentity {
                            symbol: resource.binding.symbol.clone(),
                            shader_register: resource.binding.shader_register,
                            owner_packet: resource.binding.owner_packet.clone(),
                            payload_sha256: resource.binding.payload_sha256.clone(),
                            descriptor_sha256: resource.binding.descriptor_sha256.clone(),
                        },
                        resource.as_raw(),
                    )
                }
            })
            .collect()
    }
}

impl ProductPoliticalHandoffRuntime {
    fn retain_frame(
        &mut self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
    ) -> Result<&map_viewer::Xmap05D3dHandoffReceipt> {
        let political = &prepared.map_modes.render_political;
        political.validate()?;
        if self.handoff.is_none() {
            self.handoff = Some(unsafe {
                map_viewer::NativeXmap05PoliticalHandoff::create_on_device(
                    gfx.device.as_raw(),
                    political,
                )?
            });
        } else {
            unsafe {
                self.handoff
                    .as_mut()
                    .ok_or("XMAP05 political handoff disappeared")?
                    .update_frame(gfx.device.as_raw(), political)?;
            }
        }
        Ok(self
            .handoff
            .as_ref()
            .ok_or("XMAP05 political handoff was not retained")?
            .receipt())
    }

    fn xmap16_rail_resources(
        &self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
    ) -> Result<Vec<map_viewer::NativeXmap16RailExternalResourceView>> {
        let political = &prepared.map_modes.render_political;
        political.validate()?;
        let handoff = self
            .handoff
            .as_ref()
            .ok_or("XMAP05 resources are absent at XMAP16 rail execution")?;
        if handoff.receipt().political_frame_sha256 != political.deterministic_sha256 {
            return Err("XMAP05 rail resources belong to a different political frame".into());
        }
        [
            (
                handoff.province_secondary_color_map()?,
                &political.province_secondary,
                6,
            ),
            (
                handoff.gradient_border_channel1()?,
                &political.gradient_border_channel1,
                12,
            ),
            (
                handoff.gradient_border_channel2()?,
                &political.gradient_border_channel2,
                13,
            ),
        ]
        .into_iter()
        .map(|(resource, expected, shader_register)| {
            if resource.symbol != expected.descriptor.symbol
                || resource.resource_sha256 != expected.descriptor.payload_sha256
                || resource.descriptor_sha256 != expected.descriptor.descriptor_sha256
                || resource.width != expected.descriptor.width
                || resource.height != expected.descriptor.height
                || resource.dxgi_format != expected.descriptor.dxgi_format
            {
                return Err(format!(
                    "XMAP05 {} does not match the current prepared political texture",
                    resource.symbol
                )
                .into());
            }
            unsafe {
                require_current_srv_device(
                    gfx,
                    resource.shader_resource_view,
                    &format!("XMAP05 {}", resource.symbol),
                )?;
                map_viewer::NativeXmap16RailExternalResourceView::from_provider(
                    map_viewer::Xmap16RailExternalResourceIdentity {
                        symbol: resource.symbol.into(),
                        shader_register,
                        owner_packet: resource.owner_packet.into(),
                        payload_sha256: resource.resource_sha256.into(),
                        descriptor_sha256: resource.descriptor_sha256.into(),
                    },
                    resource.shader_resource_view,
                )
            }
        })
        .collect()
    }
}

impl ProductLightingHandoffRuntime {
    fn retain_frame(
        &mut self,
        gfx: &Gfx,
        camera: &Camera,
        prepared: &map_viewer::PreparedFoundationFrame,
    ) -> Result<Option<&map_viewer::Xmap09D3dHandoffReceipt>> {
        let Some(lighting) = require_current_lighting_foundation(camera, prepared)? else {
            self.handoff = None;
            self.shadow_plan = None;
            return Ok(None);
        };
        if self.handoff.as_ref().is_none_or(|handoff| {
            handoff.receipt().prepared_frame_sha256 != lighting.deterministic_sha256
        }) {
            self.handoff = Some(unsafe {
                map_viewer::NativeXmap09LightingHandoff::create_on_device(
                    gfx.device.as_raw(),
                    lighting,
                )?
            });
        }
        let handoff = self
            .handoff
            .as_mut()
            .ok_or("XMAP09 lighting handoff was not retained")?;
        if handoff.receipt().camera_frame_sha256 != lighting.camera_frame_sha256 {
            return Err("XMAP09 retained resources belong to a different camera frame".into());
        }
        if handoff.receipt().replay_frame_identity_sha256 != lighting.replay_frame_identity_sha256
            || handoff.receipt().replay_frame_identity_sha256
                != prepared.terrain.replay_frame_identity_sha256
        {
            return Err("XMAP09 retained resources belong to a different replay frame".into());
        }
        Ok(Some(handoff.receipt()))
    }

    fn execute_light_depth(
        &mut self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
        xmap11_bindings: Option<&ProductXmap11LightDepthBindings<'_>>,
        xmap12_bindings: Option<&ProductXmap12LightDepthBindings<'_>>,
        xmap13_bindings: Option<&ProductXmap12LightDepthBindings<'_>>,
    ) -> Result<&map_viewer::Xmap09D3dHandoffReceipt> {
        let lighting = prepared
            .lighting
            .as_ref()
            .ok_or("XMAP09 light-depth execution has no current lighting frame")?;
        let handoff = self
            .handoff
            .as_mut()
            .ok_or("XMAP09 light-depth execution has no retained handoff")?;
        if handoff.receipt().prepared_frame_sha256 != lighting.deterministic_sha256 {
            return Err("XMAP09 light-depth handoff belongs to another prepared frame".into());
        }
        let xmap12 = require_current_xmap12_foundation(prepared)?;
        if xmap12.is_some() != xmap12_bindings.is_some() {
            return Err("XMAP12 L03 frame/provider availability disagrees".into());
        }
        let xmap13_has_casters = prepared
            .units
            .as_ref()
            .is_some_and(|frame| !frame.light_depth_casters.is_empty());
        if xmap13_has_casters != xmap13_bindings.is_some() {
            return Err("XMAP13 L03 frame/provider availability disagrees".into());
        }
        let tree_frame = prepared
            .trees
            .as_ref()
            .ok_or("XMAP11 complete foundation omitted the prepared tree frame")?;
        if xmap11_bindings.is_none() {
            return Err("XMAP11 L03 frame/provider availability disagrees".into());
        }
        let enabled_partition_mask = xmap12_bindings
            .map(|bindings| bindings.enabled_partition_mask)
            .unwrap_or(0)
            | xmap13_bindings
                .map(|bindings| bindings.enabled_partition_mask)
                .unwrap_or(0)
            | tree_frame
                .selected_batches
                .iter()
                .fold(0u8, |mask, batch| mask | batch.partition_mask);
        let enabled_partition_mask = if enabled_partition_mask == 0 {
            map_viewer::L03_PARTITION_PRIMARY
        } else {
            enabled_partition_mask
        };
        let mut pdxmesh_rows = xmap12
            .map(|frame| frame.ordered_l03_rows.clone())
            .unwrap_or_default();
        pdxmesh_rows.extend(
            prepared
                .units
                .as_ref()
                .map(|frame| frame.light_depth_casters.clone())
                .unwrap_or_default(),
        );
        let projection = lighting.project_exact_casters(
            enabled_partition_mask,
            pdxmesh_rows,
            tree_frame.l03_shadow_batches()?,
        )?;
        let active = if projection.inactive_excluded || projection.submissions.is_empty() {
            let mut casters: Vec<&mut dyn map_viewer::NativeXmap09LightDepthCaster> = Vec::new();
            unsafe {
                handoff
                    .execute_light_depth(gfx.context.as_raw(), &projection, &mut casters)?
                    .active
            }
        } else {
            let tree_bindings = xmap11_bindings
                .ok_or("L03 projection became active without the XMAP11 native caster")?;
            let mut tree_caster = map_viewer::NativeXmap11LightDepthCaster::new(
                gfx.device.as_raw(),
                tree_bindings.owner,
                tree_bindings.frame,
                &lighting.shadow.selected_light_frame,
                &projection,
                tree_bindings.partitions,
            )?;
            let mut xmap12_caster = xmap12_bindings
                .map(|bindings| unsafe {
                    map_viewer::NativeXmap12LightDepthCaster::new(
                        gfx.device.as_raw(),
                        bindings.owned,
                        bindings.frame,
                        &projection,
                        &lighting.shadow.selected_light_frame,
                        bindings.programs,
                        bindings.partitions,
                    )
                })
                .transpose()?;
            let mut xmap13_caster = xmap13_bindings
                .map(|bindings| unsafe {
                    map_viewer::NativeXmap12LightDepthCaster::new_xmap13(
                        gfx.device.as_raw(),
                        bindings.owned,
                        bindings.frame,
                        &projection,
                        &lighting.shadow.selected_light_frame,
                        bindings.programs,
                        bindings.partitions,
                    )
                })
                .transpose()?;
            let mut casters: Vec<&mut dyn map_viewer::NativeXmap09LightDepthCaster> = Vec::new();
            if let Some(caster) = xmap12_caster.as_mut() {
                casters.push(caster);
            }
            if let Some(caster) = xmap13_caster.as_mut() {
                casters.push(caster);
            }
            casters.push(&mut tree_caster);
            unsafe {
                handoff
                    .execute_light_depth(gfx.context.as_raw(), &projection, &mut casters)?
                    .active
            }
        };
        if active {
            let depth = handoff.light_depth_view()?;
            if depth.frame_identity_sha256() != lighting.camera_frame_sha256 {
                return Err("XMAP09 executed light depth belongs to another camera frame".into());
            }
            if depth.replay_frame_identity_sha256() != lighting.replay_frame_identity_sha256 {
                return Err("XMAP09 executed light depth belongs to another replay frame".into());
            }
            self.shadow_plan = Some(
                lighting
                    .plan_shadow_consumer(&projection)?
                    .ok_or("XMAP09 active light depth omitted the XMAP10 screen-shadow plan")?,
            );
        } else {
            self.shadow_plan = None;
        }
        Ok(handoff.receipt())
    }
}

impl ProductTreeHandoffRuntime {
    fn retain_frame(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        prepared: &map_viewer::PreparedFoundationFrame,
    ) -> Result<&map_viewer::NativeXmap11ResourceOwner> {
        self.execution = None;
        let frame = prepared
            .trees
            .as_ref()
            .ok_or("XMAP11 complete foundation omitted the prepared tree frame")?;
        frame.validate()?;
        if self
            .owner
            .as_ref()
            .is_none_or(|owner| !owner.matches(gfx.device.as_raw(), frame))
        {
            self.owner = Some(unsafe {
                map_viewer::NativeXmap11ResourceOwner::create_on_device(
                    gfx.device.as_raw(),
                    resolver,
                    admission,
                    frame,
                )?
            });
        }
        self.owner
            .as_ref()
            .ok_or_else(|| "XMAP11 native resource owner disappeared".into())
    }

    fn owner(&self) -> Result<&map_viewer::NativeXmap11ResourceOwner> {
        self.owner
            .as_ref()
            .ok_or_else(|| "XMAP11 native resource owner is unavailable".into())
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn execute_main_scene(
        &mut self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        schedule: &map_viewer::AssignedStage,
        target: &map_viewer::NativeProductionMainSceneTarget,
        scene_cb0s: &[map_viewer::NativeXmap10MapCopySceneCb0View<'_>],
        political: &map_viewer::NativeXmap05PoliticalHandoff,
        lighting: &map_viewer::NativeXmap09LightingHandoff,
        water: &map_viewer::NativeXmap07WaterHandoff,
        screen_shadow: &map_viewer::NativeXmap10ScreenShadowHandoff,
    ) -> Result<&map_viewer::TreeNativeExecutionReceipt> {
        let frame = prepared
            .trees
            .as_ref()
            .ok_or("XMAP11 MainScene execution has no prepared tree frame")?;
        let viewer_state_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(viewer)?);
        if viewer_state_sha256 != frame.viewer_state_sha256 {
            return Err("XMAP11 MainScene viewer differs from its prepared frame".into());
        }
        let owner = self.owner()?;
        if !owner.matches(gfx.device.as_raw(), frame) {
            return Err("XMAP11 MainScene resource owner is stale".into());
        }
        let make = |semantic: &str, slot: u8, payload: &str, descriptor: &str, view| {
            map_viewer::NativeTreeShaderResource {
                identity: map_viewer::TreeGpuResourceIdentity {
                    semantic: semantic.into(),
                    slot,
                    content_payload_sha256: payload.into(),
                    descriptor_sha256: descriptor.into(),
                },
                view,
            }
        };
        let mut shared = owner.private_shared_resources();
        let color = political.shader_resource("ProvincePrimaryColorMap")?;
        shared.push(make(
            "ColorMap",
            4,
            color.resource_sha256,
            color.descriptor_sha256,
            color.shader_resource_view,
        ));
        if frame.shader_programs.main.effect_name == map_viewer::XMAP11_TREE_MAIN_EFFECT {
            let snow = water.resource_view("SnowMudTexture")?.at_shader_register(6);
            shared.push(make(
                "SnowMudData",
                6,
                &snow.binding.resource_sha256,
                &snow.binding.descriptor_sha256,
                snow.as_raw(),
            ));
            let light_index = lighting.light_index_map_view();
            shared.push(make(
                "LightIndexMap",
                10,
                &light_index.binding.resource_sha256,
                &light_index.binding.descriptor_sha256,
                light_index.as_raw(),
            ));
            let light_data = lighting.light_data_map_view();
            shared.push(make(
                "LightDataMap",
                11,
                &light_data.binding.resource_sha256,
                &light_data.binding.descriptor_sha256,
                light_data.as_raw(),
            ));
        }
        let shadow = screen_shadow.shadow_map_view()?.at_shader_register(8);
        shared.push(make(
            "ShadowMap",
            8,
            shadow.resource_sha256(),
            shadow.descriptor_sha256(),
            shadow.as_raw(),
        ));
        for (semantic, slot, resource) in [
            (
                "GradientBorderChannel1",
                12,
                political.gradient_border_channel1()?,
            ),
            (
                "GradientBorderChannel2",
                13,
                political.gradient_border_channel2()?,
            ),
            (
                "ProvinceSecondaryColorMap",
                15,
                political.province_secondary_color_map()?,
            ),
        ] {
            shared.push(make(
                semantic,
                slot,
                resource.resource_sha256,
                resource.descriptor_sha256,
                resource.shader_resource_view,
            ));
        }
        shared.sort_by_key(|row| row.identity.slot);
        let partition_scenes = scene_cb0s
            .iter()
            .map(|scene| map_viewer::NativeTreePartitionSceneBinding {
                render_view_mask: scene.render_view_mask(),
                map_copy_offset_f32_bits: scene.map_copy_offset_f32_bits(),
                viewer_state_sha256: viewer_state_sha256.clone(),
                scene_cb0_payload_sha256: scene.payload_sha256().to_owned(),
                scene_cb0: scene.raw_buffer(),
            })
            .collect();
        let point_lights = lighting.point_lights_cb2_view();
        let pixel_cb2 =
            if frame.shader_programs.main.effect_name == map_viewer::XMAP11_TREE_MAIN_EFFECT {
                point_lights.as_raw()
            } else {
                std::ptr::null_mut()
            };
        let bindings = map_viewer::NativeTreeMainSceneBindings {
            color_rtv: target.color_rtv(),
            depth_dsv: target.depth_dsv(),
            width: target.desc().color_width,
            height: target.desc().color_height,
            ordered_partition_scene_cb0: partition_scenes,
            pixel_cb2,
            shadow_map_texture_matrix_f32_bits: lighting
                .shadow_map_texture_matrix()
                .shadow_map_texture_matrix_f32_bits,
            ordered_shared_resources: shared,
            ordered_asset_resources: owner.asset_resources(),
        };
        let authority = map_viewer::TreeNativeExecutionAuthority {
            hour_index: prepared.envelope.hour_index,
            replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
            viewer_state_sha256,
            main_scene_target_sha256: target.resource_identity_sha256().to_owned(),
            map_copy_receipt_set_sha256: frame.map_copy_receipt_set_sha256.clone(),
            foundation_authorization_sha256: map_viewer::xmap11_foundation_authorization_sha256(
                frame, schedule,
            )?,
            color_map_second_is_null: true,
        };
        let receipt = map_viewer::execute_xmap11_tree_frame(
            gfx.device.as_raw(),
            gfx.context.as_raw(),
            frame,
            schedule,
            &authority,
            owner,
            &bindings,
        )?;
        receipt.validate_against(frame, schedule, &authority)?;
        self.execution = Some(receipt);
        Ok(self
            .execution
            .as_ref()
            .expect("XMAP11 execution receipt was just stored"))
    }

    fn completed_receipt(&self) -> Option<&map_viewer::TreeNativeExecutionReceipt> {
        self.execution.as_ref()
    }
}

impl ProductXmap12HandoffRuntime {
    fn retain_owned_resources(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        prepared: &map_viewer::PreparedFoundationFrame,
        map_width: u32,
        map_height: u32,
    ) -> Result<()> {
        let Some(frame) = require_current_xmap12_foundation(prepared)? else {
            self.owned = None;
            self.execution = ProductXmap12ExecutionStatus::Unavailable {
                reason: "authenticated XMAP12 per-frame carriers are unavailable".into(),
            };
            return Ok(());
        };
        if self.owned.as_ref().is_none_or(|owned| {
            !owned.matches(
                gfx.device.as_raw(),
                frame,
                map_width,
                map_height,
                gfx.width,
                gfx.height,
            )
        }) {
            self.execution = ProductXmap12ExecutionStatus::Unavailable {
                reason: "current XMAP12 resources are retained but MainScene has not executed"
                    .into(),
            };
            self.owned = Some(unsafe {
                map_viewer::NativeXmap12ResourceOwner::create_on_device(
                    gfx.device.as_raw(),
                    resolver,
                    admission,
                    frame,
                    map_width,
                    map_height,
                    gfx.width,
                    gfx.height,
                )?
            });
        }
        Ok(())
    }

    fn owned(&self) -> Result<&map_viewer::NativeXmap12ResourceOwner> {
        self.owned
            .as_ref()
            .ok_or("XMAP12 has no retained current-frame native owner".into())
    }

    fn mark_provider_unavailable(&mut self, reason: impl Into<String>) {
        self.execution = ProductXmap12ExecutionStatus::Unavailable {
            reason: reason.into(),
        };
    }

    fn execution_complete(&self) -> bool {
        self.completed_receipt().is_some()
    }

    fn completed_receipt(&self) -> Option<&map_viewer::Xmap12NativeExecutionReceipt> {
        match &self.execution {
            ProductXmap12ExecutionStatus::AuthenticatedEmpty(receipt)
            | ProductXmap12ExecutionStatus::Present(receipt) => Some(receipt),
            ProductXmap12ExecutionStatus::Unavailable { .. } => None,
        }
    }

    fn unavailable_reason(&self) -> Option<&str> {
        match &self.execution {
            ProductXmap12ExecutionStatus::Unavailable { reason } => Some(reason),
            ProductXmap12ExecutionStatus::AuthenticatedEmpty(_)
            | ProductXmap12ExecutionStatus::Present(_) => None,
        }
    }

    fn execute_main_scene(
        &mut self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
        bindings: &ProductXmap12MainSceneBindings<'_>,
    ) -> Result<&ProductXmap12ExecutionStatus> {
        let frame = require_current_xmap12_foundation(prepared)?
            .ok_or("XMAP12 MainScene execution requires a current prepared XMAP12 frame")?;
        let owned = self
            .owned
            .as_ref()
            .ok_or("XMAP12 MainScene execution has no retained owned resources")?;
        let receipt = unsafe {
            map_viewer::execute_xmap12_main_scene(
                gfx.device.as_raw(),
                gfx.context.as_raw(),
                owned,
                frame,
                bindings.native,
            )?
        };
        receipt.validate()?;
        if receipt.prepared_frame_sha256 != frame.deterministic_sha256
            || receipt.content_manifest_sha256 != prepared.envelope.content_manifest_sha256
            || receipt.viewer_state_sha256 != frame.viewer_state_sha256
        {
            return Err(
                "XMAP12 native MainScene receipt belongs to different foundation input".into(),
            );
        }
        self.execution = if receipt.issued_draw_count == 0 {
            ProductXmap12ExecutionStatus::AuthenticatedEmpty(receipt)
        } else {
            ProductXmap12ExecutionStatus::Present(receipt)
        };
        Ok(&self.execution)
    }
}

impl ProductXmap13UnitRuntime {
    #[allow(clippy::too_many_arguments)]
    fn retain_owned_resources(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        content_manifest: &replay_core::ContentManifest,
        prepared: &map_viewer::PreparedFoundationFrame,
        texture_quality: u8,
        map_width: u32,
        map_height: u32,
    ) -> Result<()> {
        require_current_units_foundation(prepared)?;
        let Some(units) = prepared.units.as_ref() else {
            self.owned = None;
            self.static_content = None;
            self.shadow_bridge = None;
            self.execution = ProductXmap13UnitExecutionStatus::Unavailable {
                reason: "authenticated XMAP13 semantic unit census is unavailable".into(),
            };
            return Ok(());
        };
        if texture_quality > 2 {
            return Err("XMAP13 texture quality is outside exact engine domain 0..=2".into());
        }
        let static_content = [
            format!("map/terrain/fow_noise_{texture_quality}.dds"),
            "map/terrain/fow_rgb_waterspec_a.dds".into(),
            "map/terrain/reflection.dds".into(),
        ]
        .map(|path| {
            admission
                .require_used_content(&path)
                .map(Clone::clone)
                .map_err(|error| -> Box<dyn std::error::Error> {
                    format!("XMAP13 requires admitted engine texture {path}: {error}").into()
                })
        });
        let [fow_noise, fow_height, environment] = static_content
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .map_err(|_| "XMAP13 static texture closure changed shape")?;
        for content in [&fow_noise, &fow_height, &environment] {
            resolver.read_admitted_used_content(content)?;
        }
        let native_viewer_sha256 = prepared.terrain.camera_frame_sha256.as_str();
        let effective_vfs_sha256 = content_manifest.effective_vfs_manifest_sha256.as_str();
        let replay_frame_sha256 = prepared.terrain.replay_frame_identity_sha256.as_str();
        let current = [fow_noise.clone(), fow_height.clone(), environment.clone()];
        let matches = self.owned.as_ref().is_some_and(|owned| {
            owned
                .matches_xmap13(
                    gfx.device.as_raw(),
                    units,
                    &prepared.envelope.content_manifest_sha256,
                    effective_vfs_sha256,
                    replay_frame_sha256,
                    native_viewer_sha256,
                    fow_noise.clone(),
                    fow_height.clone(),
                    environment.clone(),
                    map_width,
                    map_height,
                    gfx.width,
                    gfx.height,
                )
                .unwrap_or(false)
        });
        if !matches {
            self.execution = ProductXmap13UnitExecutionStatus::Unavailable {
                reason: "current XMAP13 unit resources are retained but not executed".into(),
            };
            self.owned = Some(unsafe {
                map_viewer::NativeXmap12ResourceOwner::create_xmap13_on_device(
                    gfx.device.as_raw(),
                    resolver,
                    admission,
                    units,
                    &prepared.envelope.content_manifest_sha256,
                    effective_vfs_sha256,
                    replay_frame_sha256,
                    native_viewer_sha256,
                    fow_noise,
                    fow_height,
                    environment,
                    map_width,
                    map_height,
                    gfx.width,
                    gfx.height,
                )?
            });
        }
        self.static_content = Some(current);
        let [fow_noise, fow_height, environment] = self
            .static_content
            .clone()
            .expect("XMAP13 static content stored");
        self.shadow_bridge = Some(map_viewer::xmap13_native_bridge_frame(
            units,
            &prepared.envelope.content_manifest_sha256,
            effective_vfs_sha256,
            replay_frame_sha256,
            native_viewer_sha256,
            fow_noise,
            fow_height,
            environment,
        )?);
        Ok(())
    }

    fn owned(&self) -> Result<&map_viewer::NativeXmap12ResourceOwner> {
        self.owned
            .as_ref()
            .ok_or("XMAP13 has no retained current-frame native unit owner".into())
    }

    fn shadow_bridge(&self) -> Result<&map_viewer::PreparedXmap12RenderFrame> {
        self.shadow_bridge
            .as_ref()
            .ok_or("XMAP13 has no current native light-depth bridge".into())
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn execute(
        &mut self,
        gfx: &Gfx,
        content_manifest: &replay_core::ContentManifest,
        prepared: &map_viewer::PreparedFoundationFrame,
        identity: &map_viewer::Xmap13TargetExecutionIdentity,
        bindings: &map_viewer::NativeXmap12MainSceneBindings,
    ) -> Result<&map_viewer::Xmap13NativeComponentReceipt> {
        let units = prepared
            .units
            .as_ref()
            .ok_or("XMAP13 native execution requires a current prepared unit frame")?;
        let [fow_noise, fow_height, environment] = self
            .static_content
            .clone()
            .ok_or("XMAP13 native execution lost its admitted static content")?;
        let receipt = map_viewer::execute_xmap13_unit_entities(
            gfx.device.as_raw(),
            gfx.context.as_raw(),
            self.owned()?,
            units,
            identity,
            &prepared.envelope.content_manifest_sha256,
            &content_manifest.effective_vfs_manifest_sha256,
            &prepared.terrain.replay_frame_identity_sha256,
            &prepared.terrain.camera_frame_sha256,
            fow_noise,
            fow_height,
            environment,
            bindings,
        )?;
        self.execution = ProductXmap13UnitExecutionStatus::Complete(receipt);
        match &self.execution {
            ProductXmap13UnitExecutionStatus::Complete(receipt) => Ok(receipt),
            ProductXmap13UnitExecutionStatus::Unavailable { .. } => unreachable!(),
        }
    }

    fn completed_receipt(&self) -> Option<&map_viewer::Xmap13NativeComponentReceipt> {
        match &self.execution {
            ProductXmap13UnitExecutionStatus::Complete(receipt) => Some(receipt),
            ProductXmap13UnitExecutionStatus::Unavailable { .. } => None,
        }
    }

    fn unavailable_reason(&self) -> Option<&str> {
        match &self.execution {
            ProductXmap13UnitExecutionStatus::Unavailable { reason } => Some(reason),
            ProductXmap13UnitExecutionStatus::Complete(_) => None,
        }
    }
}

impl ProductWaterHandoffRuntime {
    fn retain_frame(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        prepared: &map_viewer::PreparedFoundationFrame,
    ) -> Result<&map_viewer::Xmap07D3dHandoffReceipt> {
        self.execution = ProductWaterExecutionStatus::Unavailable {
            missing_provider: if prepared.water.underwater_required {
                "current XMAP10 aggregate SceneCB0/MainScene/ShadowMap providers have not joined"
            } else {
                "current XMAP10 aggregate SceneCB0/MainScene providers have not joined"
            },
        };
        if self.handoff.as_ref().is_some_and(|handoff| {
            handoff.receipt().water_program_sha256 != prepared.water.program.deterministic_sha256
                || handoff.resource_view("WaterRefraction").is_ok_and(|view| {
                    view.binding.descriptor_sha256
                        != map_viewer::xmap04_underwater_srv_descriptor_sha256(
                            prepared.water.refraction_dxgi_format,
                            4,
                            0,
                            1,
                            0,
                            1,
                        )
                })
        }) {
            self.handoff = None;
        }
        if self.handoff.is_none() {
            self.handoff = Some(unsafe {
                map_viewer::NativeXmap07WaterHandoff::create_on_device(
                    gfx.device.as_raw(),
                    resolver,
                    &prepared.water,
                )?
            });
        } else {
            unsafe {
                self.handoff
                    .as_mut()
                    .ok_or("XMAP07 water handoff disappeared")?
                    .update_frame(gfx.device.as_raw(), &prepared.water)?;
            }
        }
        Ok(self
            .handoff
            .as_ref()
            .ok_or("XMAP07 water handoff was not retained")?
            .receipt())
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    unsafe fn execute_complete_high_gfx_frame<R>(
        &mut self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        target: &map_viewer::NativeProductionMainSceneTarget,
        terrain: &mut ProductTerrainHandoffRuntime,
        political: &ProductPoliticalHandoffRuntime,
        lighting: &ProductLightingHandoffRuntime,
        scene_cb0s: &[&map_viewer::NativeXmap10MapCopySceneCb0View<'_>],
        shadow_map: &R,
    ) -> Result<&ProductWaterExecutionStatus>
    where
        R: map_viewer::NativeXmap04UnderwaterShaderResource,
    {
        self.execution = ProductWaterExecutionStatus::Unavailable {
            missing_provider: "current XMAP07 draw sequence has not completed",
        };
        if prepared.water.program.effect_name != "water" || !prepared.water.underwater_required {
            return Err("XMAP07 high-GFX executor received a low-GFX prepared water frame".into());
        }
        let map_copy_receipts = prepared.terrain.ordered_map_copy_receipts()?;
        let map_copy_receipt_set_sha256 = prepared.terrain.map_copy_receipt_set_sha256()?;
        if scene_cb0s.len() != map_copy_receipts.len()
            || scene_cb0s
                .iter()
                .zip(&map_copy_receipts)
                .any(|(scene, row)| {
                    scene.map_copy_receipt_set_sha256() != map_copy_receipt_set_sha256
                        || scene.render_view_mask() != row.render_view_mask
                        || scene.map_copy_offset_f32_bits() != row.map_copy_offset_f32_bits
                })
        {
            return Err(
                "XMAP07 SceneCB0 providers do not match the authenticated XMAP04 map-copy order"
                    .into(),
            );
        }
        let main_scene_cb0 = scene_cb0s
            .iter()
            .find(|scene| scene.render_view_mask() == 1)
            .copied()
            .ok_or("XMAP07 high-GFX execution omitted the main-map SceneCB0")?;

        let water = self
            .handoff
            .as_ref()
            .ok_or("XMAP07 water handoff is absent at surface execution")?;
        let terrain = terrain
            .handoff
            .as_mut()
            .ok_or("XMAP04 terrain handoff is absent at water execution")?;
        let political = political
            .handoff
            .as_ref()
            .ok_or("XMAP05 political handoff is absent at water execution")?;
        let lighting = lighting
            .handoff
            .as_ref()
            .ok_or("XMAP09 lighting handoff is absent at water execution")?;

        let light_index = lighting.light_index_map_view().at_shader_register(10);
        let light_data = lighting.light_data_map_view().at_shader_register(11);
        if shadow_map.owner_packet() != "XMAP10"
            || shadow_map.symbol() != "ShadowMap"
            || shadow_map.shader_register() != 12
        {
            return Err("XMAP07 received the wrong XMAP10 ShadowMap provider".into());
        }
        let point_lights = lighting.point_lights_cb2_view();
        let point_lights_surface =
            map_viewer::NativeXmap07PointLightsConstantBufferView::from_xmap09(&point_lights)?;
        let underwater_target = water.refraction_render_target(&prepared.water, target.desc())?;
        let light_resources: [&dyn map_viewer::NativeXmap04UnderwaterShaderResource; 3] =
            [&light_index, &light_data, shadow_map];
        let underwater = terrain.render_underwater_refraction(
            gfx.context.as_raw(),
            &prepared.terrain,
            &underwater_target,
            main_scene_cb0,
            &point_lights,
            &light_resources,
        )?;

        let replay_frame = prepared.water.replay_frame_identity_sha256.clone();
        let candidates = vec![
            map_viewer::NativeXmap07DependentResourceView::from_xmap05(
                political.province_secondary_color_map()?,
                replay_frame.clone(),
            )?,
            map_viewer::NativeXmap07DependentResourceView::from_camera_owner(&light_index)?,
            map_viewer::NativeXmap07DependentResourceView::from_camera_owner(&light_data)?,
            map_viewer::NativeXmap07DependentResourceView::from_xmap05(
                political.gradient_border_channel1()?,
                replay_frame.clone(),
            )?,
            map_viewer::NativeXmap07DependentResourceView::from_xmap05(
                political.gradient_border_channel2()?,
                replay_frame.clone(),
            )?,
            map_viewer::NativeXmap07DependentResourceView::from_xmap05(
                political.gradient_border_channel3()?,
                replay_frame,
            )?,
            map_viewer::NativeXmap07DependentResourceView::from_camera_owner(shadow_map)?,
        ];
        let mut ordered_dependencies = Vec::new();
        for sampler in prepared
            .water
            .program
            .samplers
            .iter()
            .filter(|sampler| sampler.owner_packet != "XMAP07")
        {
            ordered_dependencies.push(
                candidates
                    .iter()
                    .find(|view| {
                        view.binding.symbol == sampler.symbol
                            && view.binding.owner_packet == sampler.owner_packet
                    })
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "XMAP07 app execution omitted dependency {} from {}",
                            sampler.symbol, sampler.owner_packet
                        )
                    })?,
            );
        }

        let mut surface_draws = Vec::with_capacity(map_copy_receipts.len());
        for (draw_ordinal, (scene_cb0, map_copy_receipt)) in
            scene_cb0s.iter().zip(&map_copy_receipts).enumerate()
        {
            surface_draws.push(water.draw_surface(
                gfx.context.as_raw(),
                &prepared.water,
                map_viewer::NativeXmap07SurfaceDrawInputs {
                    target,
                    foundation: &prepared.envelope,
                    viewer,
                    scene_cb0: *scene_cb0,
                    point_lights_cb2: Some(&point_lights_surface),
                    dependent_resources: &ordered_dependencies,
                    map_copy_receipts: &map_copy_receipts,
                    underwater_terrain_frame: Some(&prepared.terrain),
                    underwater_draw_receipt: Some(&underwater),
                    draw_ordinal: u32::try_from(draw_ordinal)?,
                    map_copy_offset_f32_bits: map_copy_receipt.map_copy_offset_f32_bits,
                },
            )?);
        }
        self.execution = ProductWaterExecutionStatus::Present {
            underwater,
            surface_draws,
        };
        Ok(&self.execution)
    }

    fn execution_present(&self) -> bool {
        matches!(
            &self.execution,
            ProductWaterExecutionStatus::Present {
                underwater,
                surface_draws,
            } if underwater.draw_count > 0 && !surface_draws.is_empty()
        )
    }

    fn unavailable_reason(&self) -> Option<&'static str> {
        match self.execution {
            ProductWaterExecutionStatus::Unavailable { missing_provider } => Some(missing_provider),
            ProductWaterExecutionStatus::Present { .. } => None,
        }
    }

    fn completed_scene_receipt_sha256(&self) -> Result<String> {
        match &self.execution {
            ProductWaterExecutionStatus::Present {
                underwater,
                surface_draws,
            } => {
                underwater.validate()?;
                let mut bytes = Vec::new();
                bytes.extend_from_slice(underwater.deterministic_sha256.as_bytes());
                for draw in surface_draws {
                    draw.validate()?;
                    bytes.extend_from_slice(draw.packet_draw_receipt_sha256.as_bytes());
                }
                Ok(snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(
                    &bytes,
                )))
            }
            ProductWaterExecutionStatus::Unavailable { .. } => {
                Err("XMAP10 post-FX cannot consume unavailable XMAP07 scene execution".into())
            }
        }
    }

    fn viewer_state_sha256(&self) -> Result<&str> {
        let ProductWaterExecutionStatus::Present { surface_draws, .. } = &self.execution else {
            return Err("XMAP10 post-FX has no completed XMAP07 viewer identity".into());
        };
        let first = surface_draws
            .first()
            .ok_or("XMAP10 post-FX has no XMAP07 surface draw")?;
        if surface_draws
            .iter()
            .any(|draw| draw.viewer_state_sha256 != first.viewer_state_sha256)
        {
            return Err("XMAP07 surface draws mix viewer identities".into());
        }
        Ok(&first.viewer_state_sha256)
    }
}

impl ProductRiversStraitsHandoffRuntime {
    fn retain_frame(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        prepared: &map_viewer::PreparedFoundationFrame,
    ) -> Result<Option<&map_viewer::Xmap08D3dHandoffReceipt>> {
        self.execution = ProductRiversStraitsExecutionStatus::Unavailable {
            reason: "current XMAP08 compositor providers have not joined".into(),
        };
        let Some(frame) = require_current_rivers_straits_foundation(prepared)? else {
            self.handoff = None;
            self.execution = ProductRiversStraitsExecutionStatus::Unavailable {
                reason: "RiversStraits is typed unavailable for the current replay hour".into(),
            };
            return Ok(None);
        };
        if self
            .handoff
            .as_ref()
            .is_none_or(|handoff| handoff.receipt().frame_sha256 != frame.deterministic_sha256)
        {
            self.handoff = Some(unsafe {
                map_viewer::NativeXmap08RiversStraitsHandoff::create_on_device(
                    gfx.device.as_raw(),
                    resolver,
                    frame,
                )?
            });
        } else {
            unsafe {
                self.handoff
                    .as_mut()
                    .ok_or("XMAP08 handoff disappeared")?
                    .update_frame(gfx.device.as_raw(), resolver, frame)?;
            }
        }
        Ok(self.handoff.as_ref().map(|handoff| handoff.receipt()))
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn execute_complete_frame(
        &mut self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        target: &map_viewer::NativeProductionMainSceneTarget,
        terrain: &map_viewer::NativeXmap04TerrainHandoff,
        political: &map_viewer::NativeXmap05PoliticalHandoff,
        water: &map_viewer::NativeXmap07WaterHandoff,
        lighting: &map_viewer::NativeXmap09LightingHandoff,
        scene_cb0s: &[&map_viewer::NativeXmap10MapCopySceneCb0View<'_>],
        screen_shadow: &map_viewer::NativeXmap10ScreenShadowHandoff,
    ) -> Result<&ProductRiversStraitsExecutionStatus> {
        self.execution = ProductRiversStraitsExecutionStatus::Unavailable {
            reason: "current XMAP08 river and strait passes have not completed".into(),
        };
        let Some(frame) = require_current_rivers_straits_foundation(prepared)? else {
            self.handoff = None;
            self.execution = ProductRiversStraitsExecutionStatus::Unavailable {
                reason: "RiversStraits is typed unavailable for the current replay hour".into(),
            };
            return Ok(&self.execution);
        };
        let handoff = self
            .handoff
            .as_mut()
            .ok_or("XMAP08 execution has no retained native handoff")?;
        if handoff.receipt().frame_sha256 != frame.deterministic_sha256 {
            return Err("XMAP08 native handoff belongs to another prepared frame".into());
        }
        let authority = map_viewer::Xmap08ExecutionFrameAuthority::from_validated_inputs(
            &prepared.envelope,
            viewer,
            target.desc(),
            target.resource_identity_sha256(),
            &prepared.terrain,
            &prepared.map_modes.render_political,
            frame,
        )?;

        let mut resource_views: Vec<Box<dyn map_viewer::NativeXmap08RiverShaderResource + '_>> =
            Vec::new();
        for sampler in frame
            .river_program
            .retained_sampler_requirements()?
            .into_iter()
            .filter(|sampler| sampler.owner_packet != "XMAP08" && sampler.owner_packet != "XMAP05")
        {
            let view: Box<dyn map_viewer::NativeXmap08RiverShaderResource + '_> =
                match (sampler.owner_packet.as_str(), sampler.name.as_str()) {
                    ("XMAP04", _) => Box::new(terrain.underwater_static_resource(
                        &prepared.terrain,
                        &sampler.name,
                        sampler.shader_register,
                    )?),
                    ("XMAP07", _) => Box::new(
                        water
                            .resource_view(&sampler.name)?
                            .at_shader_register(sampler.shader_register),
                    ),
                    ("XMAP09", "LightIndexMap") => Box::new(
                        lighting
                            .light_index_map_view()
                            .at_shader_register(sampler.shader_register),
                    ),
                    ("XMAP09", "LightDataMap") => Box::new(
                        lighting
                            .light_data_map_view()
                            .at_shader_register(sampler.shader_register),
                    ),
                    ("XMAP10", "ShadowMap") => Box::new(
                        screen_shadow
                            .shadow_map_view()?
                            .at_shader_register(sampler.shader_register),
                    ),
                    (owner, symbol) => {
                        return Err(format!(
                            "XMAP08 app has no exact provider for {owner} resource {symbol}"
                        )
                        .into())
                    }
                };
            resource_views.push(view);
        }
        let resource_refs = resource_views
            .iter()
            .map(|resource| resource.as_ref())
            .collect::<Vec<_>>();
        let scene_refs = scene_cb0s
            .iter()
            .map(|scene| *scene as &dyn map_viewer::NativeXmap08SceneConstantBuffer)
            .collect::<Vec<_>>();
        let temporal = map_viewer::Xmap08RiverTemporalBinding::from_xmap07(
            &prepared.water,
            &frame.camera_view_sha256,
            &prepared.terrain.replay_frame_identity_sha256,
        )?;
        let lighting_frame = prepared
            .lighting
            .as_ref()
            .ok_or("XMAP08 high-GFX provider join has no current XMAP09 frame")?;
        let shadow_matrix =
            map_viewer::Xmap08RiverShadowMatrixBinding::from_xmap09(lighting_frame)?;
        let point_lights = lighting.point_lights_cb2_view();
        let needs_point_lights = frame
            .river_program
            .retains_constant_buffer_owner(2, "XMAP09")?;
        let providers = map_viewer::NativeXmap08RiverProviderInputs {
            shader_resources: &resource_refs,
            scene_cb0_by_map_copy: &scene_refs,
            point_lights_cb2: needs_point_lights.then_some(
                &point_lights as &dyn map_viewer::NativeXmap04UnderwaterPointLightsConstantBuffer,
            ),
            temporal: &temporal,
            shadow_matrix: Some(&shadow_matrix),
        };
        handoff.execute_rivers(
            gfx.context.as_raw(),
            target,
            viewer,
            &authority,
            political,
            &providers,
        )?;
        handoff.execute_straits(
            gfx.context.as_raw(),
            target,
            viewer,
            &authority,
            Some(political),
        )?;
        handoff
            .receipt()
            .validate_complete_for_authority(&frame.deterministic_sha256, &authority)?;
        self.execution = ProductRiversStraitsExecutionStatus::Present(handoff.receipt().clone());
        Ok(&self.execution)
    }

    fn completed_scene_receipt_sha256(&self) -> Result<&str> {
        match &self.execution {
            ProductRiversStraitsExecutionStatus::Present(receipt) => {
                Ok(&receipt.deterministic_sha256)
            }
            ProductRiversStraitsExecutionStatus::Unavailable { reason } => Err(format!(
                "XMAP10 post-FX cannot consume unavailable XMAP08 scene execution: {reason}"
            )
            .into()),
        }
    }
}

impl ProductThinBorderHandoffRuntime {
    fn retain_frame(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        prepared: &map_viewer::PreparedFoundationFrame,
    ) -> Result<Option<&map_viewer::Xmap06D3dHandoffReceipt>> {
        self.early_draw = None;
        self.late_draw = None;
        let Some(frame) = require_current_thin_borders_foundation(prepared)? else {
            self.handoff = None;
            return Ok(None);
        };
        if self.handoff.as_ref().is_none_or(|handoff| {
            handoff.receipt().prepared_frame_sha256 != frame.deterministic_sha256
        }) {
            self.handoff = Some(unsafe {
                map_viewer::NativeXmap06ThinBorderHandoff::create_on_device(
                    gfx.device.as_raw(),
                    resolver,
                    frame,
                )?
            });
        } else {
            unsafe {
                self.handoff
                    .as_mut()
                    .ok_or("XMAP06 handoff disappeared")?
                    .update_frame(gfx.device.as_raw(), resolver, frame)?;
            }
        }
        Ok(self.handoff.as_ref().map(|handoff| handoff.receipt()))
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn execute_phase(
        &mut self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        target: &map_viewer::NativeProductionMainSceneTarget,
        main_scene_cb0: &map_viewer::NativeXmap10MapCopySceneCb0View<'_>,
        screen_shadow: &map_viewer::NativeXmap10ScreenShadowHandoff,
        phase: map_viewer::Xmap06BorderExecutionPhase,
    ) -> Result<&map_viewer::Xmap06PhaseDrawReceipt> {
        let frame = require_current_thin_borders_foundation(prepared)?.ok_or(
            "XMAP06 border execution is unavailable without its authenticated runtime partition",
        )?;
        let handoff = self
            .handoff
            .as_ref()
            .ok_or("XMAP06 execution has no retained native handoff")?;
        if handoff.receipt().prepared_frame_sha256 != frame.deterministic_sha256 {
            return Err("XMAP06 native handoff belongs to another prepared frame".into());
        }
        if main_scene_cb0.render_view_mask() != 1
            || main_scene_cb0.main_scene_resource_identity_sha256()
                != target.resource_identity_sha256()
        {
            return Err("XMAP06 execution received a non-main or stale SceneCB0".into());
        }
        let camera = viewer
            .camera
            .as_ref()
            .ok_or("XMAP06 execution has no viewer-owned camera")?;
        let viewer_state_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(viewer)?);
        let identity = map_viewer::Xmap06PhaseExecutionIdentity {
            hour_index: prepared.envelope.hour_index,
            active_view_mask: 1,
            content_manifest_sha256: prepared.envelope.content_manifest_sha256.clone(),
            foundation_content_operation_sha256: prepared
                .envelope
                .foundation_content_operation_sha256
                .clone(),
            viewer_state_sha256,
            main_scene_target_sha256: snapshot::sha256::Sha256::hex(&serde_json::to_vec(
                target.desc(),
            )?),
            map_copy_receipt_set_sha256: main_scene_cb0.map_copy_receipt_set_sha256().to_owned(),
        };
        let viewport = winapi::um::d3d11::D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: target.desc().color_width as f32,
            Height: target.desc().color_height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let shadow_map = screen_shadow.shadow_map_view()?;
        let receipt = handoff.execute_phase(
            gfx.context.as_raw(),
            target.color_rtv(),
            target.depth_dsv(),
            &[viewport],
            main_scene_cb0.raw_buffer(),
            main_scene_cb0.raw_buffer(),
            None,
            None,
            shadow_map.as_raw(),
            camera.current_eye()[1],
            main_scene_cb0.application_elapsed_seconds(),
            phase,
            identity,
        )?;
        receipt.validate()?;
        let slot = match phase {
            map_viewer::Xmap06BorderExecutionPhase::Early => &mut self.early_draw,
            map_viewer::Xmap06BorderExecutionPhase::Late => &mut self.late_draw,
        };
        if slot.is_some() {
            return Err(
                "XMAP06 border phase executed more than once for the retained frame".into(),
            );
        }
        *slot = Some(receipt);
        Ok(slot.as_ref().expect("XMAP06 phase receipt was just stored"))
    }

    fn completed_scene_receipt_sha256(&self) -> Result<String> {
        let early = self
            .early_draw
            .as_ref()
            .ok_or("XMAP06 early border phase did not execute")?;
        let late = self
            .late_draw
            .as_ref()
            .ok_or("XMAP06 late border phase did not execute")?;
        early.validate()?;
        late.validate()?;
        if early.phase != map_viewer::Xmap06BorderExecutionPhase::Early
            || late.phase != map_viewer::Xmap06BorderExecutionPhase::Late
            || early.prepared_frame_sha256 != late.prepared_frame_sha256
            || early.identity != late.identity
        {
            return Err("XMAP06 completed phases do not form one exact frame".into());
        }
        Ok(snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(
            "hoi4.xmap06.completed-border-scene.v1",
            &early.packet_draw_receipt_sha256,
            &late.packet_draw_receipt_sha256,
        ))?))
    }
}

impl ProductXmap10HandoffRuntime {
    fn discard_unpresented_temporal_frame(&mut self) {
        self.main_scene = None;
        self.pre_particle_scene = None;
        self.scene_cb0s.clear();
        self.screen_shadow = None;
        self.terrain_draw = None;
        self.map_label_draw = None;
        self.xmap20_effects = None;
        self.xmap21_packet = None;
        self.xmap21_execution = None;
        self.postfx = None;
        self.temporal_receipt = None;
        self.temporal_hour_index = None;
        self.completed_postfx = None;
    }

    fn map_label_execution_complete(&self) -> bool {
        self.map_label_draw.is_some()
    }

    fn map_furniture_execution_complete(&self) -> bool {
        self.xmap21_execution.is_some()
    }

    fn map_furniture_packet(&self) -> Option<&map_viewer::PreparedMapFurniturePacket> {
        self.xmap21_packet.as_ref()
    }

    fn current_providers(&self) -> Result<ProductXmap10CurrentProviderBundle<'_>> {
        let main_scene = self
            .main_scene
            .as_ref()
            .ok_or("XMAP10 current providers have no MainScene")?;
        let pre_particle_scene = self
            .pre_particle_scene
            .as_ref()
            .ok_or("XMAP10 current providers have no pre-particle receipt")?;
        let main_scene_cb0 = self
            .scene_cb0s
            .iter()
            .find(|scene| scene.prepared().frame_identity.render_view_mask == 1)
            .ok_or("XMAP10 current providers omitted the main map-copy SceneCB0")?
            .bind_main_scene(pre_particle_scene, main_scene)?;
        let screen_shadow = self
            .screen_shadow
            .as_ref()
            .ok_or("XMAP10 current providers have no screen ShadowMap")?;
        let shadow_receipt = screen_shadow
            .receipt()
            .ok_or("XMAP10 current providers have no completed screen-shadow receipt")?;
        shadow_receipt.validate()?;
        let shadow_map = screen_shadow.shadow_map_view()?;
        Ok(ProductXmap10CurrentProviderBundle {
            main_scene,
            main_scene_cb0,
            shadow_map,
            shadow_receipt,
        })
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    unsafe fn execute_water_frame<'a>(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        content_manifest: &replay_core::ContentManifest,
        world_frame: &replay_core::WorldFrame,
        localisation_language: &str,
        delta_seconds: f32,
        transport_discontinuity: bool,
        prepared: &map_viewer::PreparedFoundationFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        political_scene_patch: &map_viewer::Xmap05SceneCb0Patch,
        presentation_scene_patch: &map_viewer::Xmap10PresentationScenePatch,
        main_scene_desc: map_viewer::ProductionMainSceneTargetDesc,
        main_scene_color: replay_core::RenderTargetDesc,
        main_scene_depth: replay_core::RenderTargetDesc,
        main_scene_producer_sha256: String,
        terrain: &mut ProductTerrainHandoffRuntime,
        political: &ProductPoliticalHandoffRuntime,
        lighting: &mut ProductLightingHandoffRuntime,
        trees: &mut ProductTreeHandoffRuntime,
        tree_schedule: &map_viewer::AssignedStage,
        xmap12: &mut ProductXmap12HandoffRuntime,
        xmap13_units: &mut ProductXmap13UnitRuntime,
        water: &'a mut ProductWaterHandoffRuntime,
        thin_borders: &mut ProductThinBorderHandoffRuntime,
        rivers_straits: &mut ProductRiversStraitsHandoffRuntime,
        orders: &mut ProductOrdersHandoffRuntime,
        rail: &mut ProductRailHandoffRuntime,
        furniture: &mut FurnitureRenderPass,
    ) -> Result<&'a ProductWaterExecutionStatus> {
        self.map_label_draw = None;
        self.xmap20_effects = None;
        self.xmap21_packet = None;
        self.xmap21_execution = None;
        prepared.envelope.validate()?;
        prepared.terrain.validate()?;
        prepared.water.validate()?;
        viewer.validate()?;
        political_scene_patch.validate()?;
        main_scene_desc.validate()?;
        if !delta_seconds.is_finite() || delta_seconds < 0.0 {
            return Err("XMAP10 production frame delta is invalid".into());
        }

        let camera = viewer
            .camera
            .as_ref()
            .ok_or("XMAP10 production execution has no viewer-owned camera")?;
        if camera.deterministic_sha256 != prepared.terrain.camera_frame_sha256
            || camera.deterministic_sha256 != prepared.water.camera_view_sha256
            || viewer.viewport_width != main_scene_desc.color_width
            || viewer.viewport_height != main_scene_desc.color_height
            || main_scene_desc.color_width != main_scene_color.width
            || main_scene_desc.color_height != main_scene_color.height
            || main_scene_desc.color_format != main_scene_color.dxgi_format_u32
            || main_scene_desc.color_samples != main_scene_color.sample_count
            || main_scene_desc.color_quality != main_scene_color.sample_quality
            || main_scene_desc.depth_width != main_scene_depth.width
            || main_scene_desc.depth_height != main_scene_depth.height
            || main_scene_desc.depth_format != main_scene_depth.dxgi_format_u32
            || main_scene_desc.depth_samples != main_scene_depth.sample_count
            || main_scene_desc.depth_quality != main_scene_depth.sample_quality
        {
            return Err(
                "XMAP10 production inputs mix a camera, viewport, or MainScene descriptor".into(),
            );
        }

        let lighting_frame = prepared
            .lighting
            .as_ref()
            .ok_or("XMAP10 production execution has no current XMAP09 lighting frame")?;
        let lighting_scene_patch = {
            let lighting_handoff = lighting
                .handoff
                .as_ref()
                .ok_or("XMAP10 production execution has no retained XMAP09 provider")?;
            if lighting_handoff.receipt().prepared_frame_sha256
                != lighting_frame.deterministic_sha256
            {
                return Err(
                    "XMAP10 XMAP09 provider differs from the prepared lighting frame".into(),
                );
            }
            lighting_handoff.scene_cb0_patch().clone()
        };

        let map_copies = prepared.terrain.ordered_map_copy_receipts()?;
        let snow_mud_input = prepared.water.scene_snow_mud_input()?;
        let mut composed = Vec::with_capacity(map_copies.len());
        for map_copy in &map_copies {
            let frame_identity = map_viewer::Xmap10SceneFrameIdentity::new(
                prepared.envelope.content_manifest_sha256.clone(),
                prepared.envelope.product_modset_sha256.clone(),
                prepared.envelope.effective_vfs_manifest_sha256.clone(),
                prepared.water.date.clone(),
                prepared.envelope.hour_index,
                prepared.terrain.map_width,
                prepared.terrain.map_height,
                viewer.viewport_width,
                viewer.viewport_height,
                prepared.terrain.replay_frame_identity_sha256.clone(),
                camera.deterministic_sha256.clone(),
                &map_copies,
                map_copy.render_view_mask,
                snow_mud_input.deterministic_sha256.clone(),
            )?;
            composed.push(map_viewer::compose_xmap10_scene_cb0(
                map_viewer::Xmap10SceneCompositionInput {
                    camera: map_viewer::Xmap03CameraScenePatch::from_xmap03(camera)?,
                    political_fow: map_viewer::Xmap05PoliticalFowScenePatch::from_xmap05(
                        political_scene_patch,
                        &frame_identity,
                    )?,
                    snow_mud: map_viewer::Xmap07SnowMudScenePatch::from_xmap07(
                        &snow_mud_input,
                        &frame_identity,
                    )?,
                    lighting: map_viewer::Xmap09LightingScenePatch::from_xmap09(
                        &lighting_scene_patch,
                        &frame_identity,
                    )?,
                    presentation: presentation_scene_patch.clone(),
                    expected_layout_sha256: prepared
                        .water
                        .program
                        .scene_cb0_reflected_layout_sha256
                        .clone(),
                    frame_identity,
                },
            )?);
        }
        let main_index = composed
            .iter()
            .position(|scene| scene.frame_identity.render_view_mask == 1)
            .ok_or("XMAP10 SceneCB0 composition omitted the main map copy")?;
        let pre_particle_scene = map_viewer::Xmap10PreParticleSceneReceipt::seal(
            prepared.envelope.hour_index,
            prepared.envelope.content_manifest_sha256.clone(),
            prepared.envelope.product_modset_sha256.clone(),
            prepared.envelope.effective_vfs_manifest_sha256.clone(),
            lighting_frame.deterministic_sha256.clone(),
            &composed[main_index],
            main_scene_color,
            main_scene_depth,
        )?;
        self.pre_particle_scene = Some(pre_particle_scene.clone());

        let had_main_scene = self.main_scene.is_some();
        let replace_main_scene = xmap10_requires_temporal_resource_reset(
            transport_discontinuity,
            self.main_scene.as_ref().is_none_or(|target| {
                target.desc() != &main_scene_desc
                    || target.device_identity() != gfx.device.as_raw() as usize
            }),
        );
        if replace_main_scene {
            self.main_scene_generation = self
                .main_scene_generation
                .checked_add(1)
                .ok_or("XMAP10 MainScene allocation generation overflowed")?;
            self.main_scene = Some(
                map_viewer::NativeProductionMainSceneTarget::create_on_device(
                    gfx.device.as_raw(),
                    main_scene_desc,
                    main_scene_producer_sha256,
                    self.main_scene_generation,
                )?,
            );
        }
        let main_scene = self
            .main_scene
            .as_ref()
            .ok_or("XMAP10 MainScene target disappeared after allocation")?;

        self.scene_cb0s = composed
            .into_iter()
            .map(|scene| {
                map_viewer::NativeXmap10SceneConstantBuffer::create_on_device(
                    gfx.device.as_raw(),
                    scene,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let scene_cb0s = self
            .scene_cb0s
            .iter()
            .map(|scene| scene.bind_main_scene(&pre_particle_scene, main_scene))
            .collect::<Result<Vec<_>>>()?;
        let scene_cb0_refs = scene_cb0s.iter().collect::<Vec<_>>();

        let xmap12_shadow_programs = if prepared.xmap12.is_some() {
            xmap12.owned()?.shadow_program_bindings()
        } else {
            Vec::new()
        };
        let xmap12_light_scene = prepared
            .xmap12
            .as_ref()
            .map(|_| unsafe {
                map_viewer::NativeXmap12LightSceneConstantBuffer::create_on_device(
                    gfx.device.as_raw(),
                    &lighting_frame.shadow.selected_light_frame,
                )
            })
            .transpose()?;
        let xmap12_submission_partition_mask = prepared
            .xmap12
            .as_ref()
            .map(|frame| {
                frame
                    .ordered_draws
                    .iter()
                    .fold(0u8, |mask, draw| mask | draw.partition_mask)
            })
            .unwrap_or(0);
        let xmap12_shadow_partitions = if let Some(light_scene) = xmap12_light_scene.as_ref() {
            [1u8, 2u8, 4u8]
                .into_iter()
                .filter(|partition| xmap12_submission_partition_mask & partition != 0)
                .map(|partition| light_scene.partition_binding(gfx.device.as_raw(), partition))
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let enabled_partition_mask = if xmap12_submission_partition_mask == 0 {
            map_viewer::L03_PARTITION_PRIMARY
        } else {
            xmap12_submission_partition_mask
        };
        let xmap12_light_bindings = prepared
            .xmap12
            .as_ref()
            .map(|_| {
                Ok::<_, Box<dyn std::error::Error>>(ProductXmap12LightDepthBindings {
                    enabled_partition_mask,
                    owned: xmap12.owned()?,
                    frame: prepared.xmap12.as_ref().expect("XMAP12 frame present"),
                    programs: &xmap12_shadow_programs,
                    partitions: &xmap12_shadow_partitions,
                })
            })
            .transpose()?;
        let xmap13_has_casters = prepared
            .units
            .as_ref()
            .is_some_and(|frame| !frame.light_depth_casters.is_empty());
        let xmap13_shadow_programs = if xmap13_has_casters {
            xmap13_units.owned()?.shadow_program_bindings()
        } else {
            Vec::new()
        };
        let xmap13_light_scene = xmap13_has_casters
            .then(|| unsafe {
                map_viewer::NativeXmap12LightSceneConstantBuffer::create_on_device(
                    gfx.device.as_raw(),
                    &lighting_frame.shadow.selected_light_frame,
                )
            })
            .transpose()?;
        let xmap13_submission_partition_mask = prepared
            .units
            .as_ref()
            .map(|frame| {
                frame
                    .native_entity_draws
                    .iter()
                    .filter(|draw| {
                        frame.light_depth_casters.iter().any(|caster| match caster {
                            map_viewer::L03SemanticInputRow::PdxmeshObject(row) => {
                                row.stable_typed_upstream_id == draw.stable_typed_id
                            }
                            map_viewer::L03SemanticInputRow::Particle { .. } => false,
                        })
                    })
                    .fold(0u8, |mask, draw| mask | draw.partition_mask)
            })
            .unwrap_or(0);
        let xmap13_shadow_partitions = if let Some(light_scene) = xmap13_light_scene.as_ref() {
            [1u8, 2u8, 4u8]
                .into_iter()
                .filter(|partition| xmap13_submission_partition_mask & partition != 0)
                .map(|partition| light_scene.partition_binding(gfx.device.as_raw(), partition))
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let xmap13_light_bindings = xmap13_has_casters
            .then(|| {
                Ok::<_, Box<dyn std::error::Error>>(ProductXmap12LightDepthBindings {
                    enabled_partition_mask: xmap13_submission_partition_mask,
                    owned: xmap13_units.owned()?,
                    frame: xmap13_units.shadow_bridge()?,
                    programs: &xmap13_shadow_programs,
                    partitions: &xmap13_shadow_partitions,
                })
            })
            .transpose()?;
        let tree_frame = prepared
            .trees
            .as_ref()
            .ok_or("XMAP10 composition omitted the current XMAP11 tree frame")?;
        let tree_partition_mask = tree_frame
            .selected_batches
            .iter()
            .fold(0u8, |mask, batch| mask | batch.partition_mask);
        let tree_light_scene = unsafe {
            map_viewer::NativeXmap12LightSceneConstantBuffer::create_on_device(
                gfx.device.as_raw(),
                &lighting_frame.shadow.selected_light_frame,
            )?
        };
        let tree_shadow_partitions = [1u8, 2u8, 4u8]
            .into_iter()
            .filter(|partition| tree_partition_mask & partition != 0)
            .map(|partition| tree_light_scene.partition_binding(gfx.device.as_raw(), partition))
            .collect::<Result<Vec<_>>>()?;
        let tree_light_bindings = ProductXmap11LightDepthBindings {
            owner: trees.owner()?,
            frame: tree_frame,
            partitions: &tree_shadow_partitions,
        };
        lighting.execute_light_depth(
            gfx,
            prepared,
            Some(&tree_light_bindings),
            xmap12_light_bindings.as_ref(),
            xmap13_light_bindings.as_ref(),
        )?;
        let lighting_handoff = lighting
            .handoff
            .as_ref()
            .ok_or("XMAP10 production execution lost its XMAP09 provider")?;

        if self.grading_table.is_none() {
            self.grading_table = Some(map_viewer::grading_law::load_posteffect_volume_table(
                resolver,
            )?);
            self.grading_defines = Some(map_viewer::grading_law::load_graphics_grading_defines(
                resolver,
            )?);
            self.region_weights =
                Some(map_viewer::grading_law::load_posteffect_region_weight_manifest(resolver)?);
        }
        let grading_table = self
            .grading_table
            .as_ref()
            .ok_or("XMAP10 grading table disappeared")?;
        let grading_defines = self
            .grading_defines
            .as_ref()
            .ok_or("XMAP10 grading defines disappeared")?;
        let region_weights = self
            .region_weights
            .as_ref()
            .ok_or("XMAP10 region weights disappeared")?;
        let camera_position =
            map_viewer::Xmap03CameraScenePatch::from_xmap03(camera)?.camera_position;
        let day_night_hour_sun_direction = lighting_frame
            .scene_constants
            .day_night_hour_sun_dir_f32_bits
            .map(f32::from_bits);
        let day_night_factor = map_viewer::grading_law::posteffect_day_night_factor_exact_a729(
            camera_position,
            prepared.terrain.map_width,
            prepared.terrain.map_height,
            day_night_hour_sun_direction,
            presentation_scene_patch.day_night_gate_factor(),
            grading_defines,
        )?;
        let region_winter_factor =
            map_viewer::grading_law::posteffect_region_winter_input_for_frame(
                resolver,
                region_weights,
                Some(world_frame.weather_province_snow()),
                camera_position,
                day_night_factor >= 0.5,
            )?
            .map(|input| input.factor())
            .unwrap_or(0.0);
        let exact_grade = grading_table.resolve_exact_grade_program(
            map_viewer::grading_law::ExactGradingFactorState {
                camera: camera_position,
                day_night_factor,
                region_winter_factor,
            },
            grading_defines,
        )?;
        let content = map_viewer::Xmap10ContentClosure::admit(
            resolver,
            content_admission,
            &exact_grade.selected_lut_paths(),
        )?;
        let active_effects = map_viewer::Xmap10ActiveEffectVariant::production_hdr(&content)?;
        let grade = map_viewer::Xmap10PerFrameGradeInputs::from_exact_grade(
            presentation_scene_patch.frame_sequence(),
            &exact_grade,
            &content,
            grading_defines,
        )?;
        let main_scene_view = scene_cb0s
            .get(main_index)
            .ok_or("XMAP10 main SceneCB0 view disappeared")?;
        let scene_receipt = map_viewer::Xmap10SceneCb0ReceiptRef {
            schema: map_viewer::XMAP10_SCENE_CB0_RECEIPT_SCHEMA.into(),
            frame_sequence: presentation_scene_patch.frame_sequence(),
            hour_index: prepared.envelope.hour_index,
            scene_cb0_sha256: main_scene_view.payload_sha256().to_owned(),
            receipt_sha256: main_scene_view.composition_receipt_sha256().to_owned(),
        };
        let temporal_is_consecutive = xmap10_temporal_hour_is_consecutive(
            self.temporal_hour_index,
            prepared.envelope.hour_index,
        );
        let temporal_source = if replace_main_scene {
            map_viewer::Xmap10TemporalAdaptationSource::EngineResourceReset {
                reason: if had_main_scene {
                    map_viewer::TemporalLuminanceResetReason::ResourcesRecreated
                } else {
                    map_viewer::TemporalLuminanceResetReason::ResourcesAllocated
                },
                main_scene_resource_identity_sha256: main_scene
                    .resource_identity_sha256()
                    .to_owned(),
                allocation_generation: main_scene.allocation_generation(),
            }
        } else if !temporal_is_consecutive {
            return Err(
                "XMAP10 temporal luminance received a nonconsecutive hour without the required engine resource-reset lifecycle"
                    .into(),
            );
        } else {
            let prior = self
                .temporal_receipt
                .as_ref()
                .ok_or("XMAP10 temporal continuation has no completed prior frame")?;
            map_viewer::Xmap10TemporalAdaptationSource::Continue {
                prior_frame_sequence: prior.frame_sequence,
                prior_receipt_sha256: prior.receipt_sha256.clone(),
            }
        };
        let temporal_receipt = map_viewer::Xmap10TemporalAdaptationReceiptRef::seal(
            presentation_scene_patch.frame_sequence(),
            temporal_source,
        )?;
        let prepared_xmap10 = map_viewer::PreparedXmap10Frame::prepare(
            prepared.envelope.hour_index,
            content,
            active_effects,
            map_viewer::Xmap10TargetPlan::new(viewer.viewport_width, viewer.viewport_height)?,
            grade,
            temporal_receipt.clone(),
            scene_receipt,
        )?;

        let terrain_handoff = terrain
            .handoff
            .as_ref()
            .ok_or("XMAP10 screen shadow has no retained XMAP04 provider")?;
        let light_depth_execution = lighting_handoff
            .receipt()
            .light_depth_execution
            .as_ref()
            .ok_or("XMAP10 screen shadow has no XMAP09 execution receipt")?;
        if light_depth_execution.active {
            let shadow_plan = lighting
                .shadow_plan
                .as_ref()
                .ok_or("XMAP10 active screen shadow has no current plan")?;
            if self.screen_shadow_plan.as_ref() != Some(shadow_plan)
                || self.screen_shadow_terrain_program_sha256.as_deref()
                    != Some(prepared.terrain.program.deterministic_sha256.as_str())
            {
                self.screen_shadow = None;
                self.screen_shadow_plan = Some(shadow_plan.clone());
                self.screen_shadow_terrain_program_sha256 =
                    Some(prepared.terrain.program.deterministic_sha256.clone());
            }
            if self.screen_shadow.is_none() {
                self.screen_shadow = Some(
                    map_viewer::NativeXmap10ScreenShadowHandoff::create_on_device(
                        gfx.device.as_raw(),
                        resolver,
                        &prepared.terrain,
                        &prepared_xmap10,
                        shadow_plan.clone(),
                    )?,
                );
            }
            let light_depth = lighting_handoff.light_depth_view()?;
            self.screen_shadow
                .as_mut()
                .ok_or("XMAP10 active screen-shadow handoff disappeared")?
                .execute(
                    gfx.context.as_raw(),
                    &prepared.terrain,
                    terrain_handoff,
                    &scene_cb0_refs,
                    lighting_handoff.shadow_map_texture_matrix(),
                    &light_depth,
                    light_depth_execution,
                )?;
        } else {
            let retained_plan = self
                .screen_shadow_plan
                .as_ref()
                .ok_or("XMAP10 inactive screen shadow has no prior active plan")?;
            let current_target = &lighting_frame.shadow.producer_and_final_target;
            if current_target.width != retained_plan.producer_and_final_target.width
                || current_target.height != retained_plan.producer_and_final_target.height
                || current_target.dxgi_format != retained_plan.producer_and_final_target.dxgi_format
                || current_target.mip_levels != retained_plan.producer_and_final_target.mip_levels
                || current_target.bind_flags != retained_plan.producer_and_final_target.bind_flags
            {
                self.screen_shadow = None;
                self.screen_shadow_plan = None;
                return Err(
                    "XMAP10 inactive screen shadow cannot retain across target reallocation".into(),
                );
            }
            self.screen_shadow
                .as_mut()
                .ok_or("XMAP10 inactive screen shadow has no defined prior target")?
                .retain_inactive(&prepared.terrain, light_depth_execution)?;
        }
        let screen_shadow = self
            .screen_shadow
            .as_mut()
            .ok_or("XMAP10 screen-shadow handoff disappeared")?;
        let shadow_map = screen_shadow.shadow_map_view()?.at_shader_register(12);
        self.terrain_draw = Some(
            terrain
                .handoff
                .as_ref()
                .ok_or("XMAP04 visible terrain has no retained native handoff")?
                .render_visible_terrain(
                    gfx.context.as_raw(),
                    &prepared.terrain,
                    main_scene,
                    political
                        .handoff
                        .as_ref()
                        .ok_or("XMAP04 visible terrain has no XMAP05 provider")?,
                    water
                        .handoff
                        .as_ref()
                        .ok_or("XMAP04 visible terrain has no XMAP07 provider")?,
                    lighting_handoff,
                    &scene_cb0_refs,
                    &shadow_map,
                )?,
        );
        if let Some(xmap12_frame) = prepared.xmap12.as_ref() {
            let programs = xmap12.owned()?.program_bindings();
            let mut required_shared = programs
                .iter()
                .flat_map(|program| program.required_shared_resources.iter().cloned())
                .collect::<Vec<_>>();
            required_shared.sort_by(|left, right| {
                left.shader_register
                    .cmp(&right.shader_register)
                    .then_with(|| left.semantic.cmp(&right.semantic))
            });
            required_shared.dedup();
            let mut shared_resources = Vec::with_capacity(required_shared.len());
            let mut missing_provider = None;
            for required in &required_shared {
                match xmap12_shared_provider(
                    required,
                    xmap12.owned()?,
                    prepared,
                    terrain,
                    political,
                    lighting,
                    water,
                    screen_shadow,
                ) {
                    ProductXmap12SharedProviderJoin::Ready(binding) => {
                        shared_resources.push(binding)
                    }
                    ProductXmap12SharedProviderJoin::Unavailable(reason) => {
                        missing_provider = Some(reason);
                        break;
                    }
                }
            }
            if let Some(reason) = missing_provider {
                xmap12.mark_provider_unavailable(reason);
            } else {
                let point_lights = lighting_handoff.point_lights_cb2_view();
                if point_lights.replay_frame_identity_sha256()
                    != xmap12_frame.replay_frame_identity_sha256
                    || point_lights.frame_identity_sha256() != prepared.terrain.camera_frame_sha256
                {
                    return Err(
                        "XMAP12 PointLightsCB2 belongs to another replay/camera frame".into(),
                    );
                }
                let partition_scenes = scene_cb0s
                    .iter()
                    .map(|scene| map_viewer::NativeXmap12PartitionSceneBinding {
                        partition_mask: scene.render_view_mask(),
                        viewer_state_sha256: xmap12_frame.viewer_state_sha256.clone(),
                        payload_sha256: scene.payload_sha256().to_owned(),
                        scene_cb0: scene.raw_buffer(),
                    })
                    .collect::<Vec<_>>();
                let native = map_viewer::NativeXmap12MainSceneBindings {
                    color_rtv: main_scene.color_rtv(),
                    depth_dsv: main_scene.depth_dsv(),
                    width: main_scene.desc().color_width,
                    height: main_scene.desc().color_height,
                    main_scene_target_sha256: pre_particle_scene.main_scene_target_sha256.clone(),
                    programs,
                    textures: xmap12.owned()?.texture_bindings(),
                    shared_resources,
                    samplers: xmap12.owned()?.sampler_bindings(),
                    partition_scenes,
                    pixel_cb2_payload_sha256: point_lights.binding.payload_sha256.clone(),
                    pixel_cb2: point_lights.as_raw(),
                };
                xmap12.execute_main_scene(
                    gfx,
                    prepared,
                    &ProductXmap12MainSceneBindings { native: &native },
                )?;
            }
        }
        if prepared.units.is_some() {
            let programs = xmap13_units.owned()?.program_bindings();
            let mut required_shared = programs
                .iter()
                .flat_map(|program| program.required_shared_resources.iter().cloned())
                .collect::<Vec<_>>();
            required_shared.sort_by(|left, right| {
                left.shader_register
                    .cmp(&right.shader_register)
                    .then_with(|| left.semantic.cmp(&right.semantic))
            });
            required_shared.dedup();
            let mut shared_resources = Vec::with_capacity(required_shared.len());
            let mut missing_provider = None;
            for required in &required_shared {
                match xmap12_shared_provider(
                    required,
                    xmap13_units.owned()?,
                    prepared,
                    terrain,
                    political,
                    lighting,
                    water,
                    screen_shadow,
                ) {
                    ProductXmap12SharedProviderJoin::Ready(binding) => {
                        shared_resources.push(binding)
                    }
                    ProductXmap12SharedProviderJoin::Unavailable(reason) => {
                        missing_provider = Some(reason);
                        break;
                    }
                }
            }
            if let Some(reason) = missing_provider {
                xmap13_units.execution = ProductXmap13UnitExecutionStatus::Unavailable { reason };
            } else {
                let point_lights = lighting_handoff.point_lights_cb2_view();
                if point_lights.replay_frame_identity_sha256()
                    != prepared.terrain.replay_frame_identity_sha256
                    || point_lights.frame_identity_sha256() != prepared.terrain.camera_frame_sha256
                {
                    return Err(
                        "XMAP13 PointLightsCB2 belongs to another replay/camera frame".into(),
                    );
                }
                let partition_scenes = scene_cb0s
                    .iter()
                    .map(|scene| map_viewer::NativeXmap12PartitionSceneBinding {
                        partition_mask: scene.render_view_mask(),
                        viewer_state_sha256: prepared.terrain.camera_frame_sha256.clone(),
                        payload_sha256: scene.payload_sha256().to_owned(),
                        scene_cb0: scene.raw_buffer(),
                    })
                    .collect::<Vec<_>>();
                let native = map_viewer::NativeXmap12MainSceneBindings {
                    color_rtv: main_scene.color_rtv(),
                    depth_dsv: main_scene.depth_dsv(),
                    width: main_scene.desc().color_width,
                    height: main_scene.desc().color_height,
                    main_scene_target_sha256: pre_particle_scene.main_scene_target_sha256.clone(),
                    programs,
                    textures: xmap13_units.owned()?.texture_bindings(),
                    shared_resources,
                    samplers: xmap13_units.owned()?.sampler_bindings(),
                    partition_scenes,
                    pixel_cb2_payload_sha256: point_lights.binding.payload_sha256.clone(),
                    pixel_cb2: point_lights.as_raw(),
                };
                let identity = map_viewer::Xmap13TargetExecutionIdentity::derive(
                    prepared,
                    viewer,
                    main_scene.desc(),
                    Some(&xmap13_counter_target_sha256(gfx)?),
                    main_scene_view.map_copy_receipt_set_sha256(),
                )?;
                xmap13_units.execute(gfx, content_manifest, prepared, &identity, &native)?;
            }
        }
        thin_borders.execute_phase(
            gfx,
            prepared,
            viewer,
            main_scene,
            main_scene_view,
            screen_shadow,
            map_viewer::Xmap06BorderExecutionPhase::Early,
        )?;
        rivers_straits.execute_complete_frame(
            gfx,
            prepared,
            viewer,
            main_scene,
            terrain
                .handoff
                .as_ref()
                .ok_or("XMAP08 execution has no retained XMAP04 provider")?,
            political
                .handoff
                .as_ref()
                .ok_or("XMAP08 execution has no retained XMAP05 provider")?,
            water
                .handoff
                .as_ref()
                .ok_or("XMAP08 execution has no retained XMAP07 provider")?,
            lighting_handoff,
            &scene_cb0_refs,
            screen_shadow,
        )?;
        let rivers_straits_scene_receipt_sha256 =
            rivers_straits.completed_scene_receipt_sha256()?.to_owned();
        let xmap16_present = matches!(
            prepared.xmap16_rail_supply_state()?,
            map_viewer::Xmap16FoundationState::Present(_)
        );
        if xmap16_present != rail.executable_frame_present() {
            return Err("XMAP16 prepared/runtime availability join drifted".into());
        }
        let supply_view_active = prepared.map_modes.xmap16_supply_view_active()?;
        let rail_execution = if xmap16_present {
            let mut political_rail_resources = political.xmap16_rail_resources(gfx, prepared)?;
            if political_rail_resources.len() != 3 {
                return Err("XMAP05 did not expose the three exact XMAP16 rail resources".into());
            }
            let province_secondary = political_rail_resources.remove(0);
            let gradient_channel1 = political_rail_resources.remove(0);
            let gradient_channel2 = political_rail_resources.remove(0);
            let rail_resources = vec![
                province_secondary,
                xmap16_water_rail_resource(gfx, prepared, water)?,
                gradient_channel1,
                gradient_channel2,
                xmap16_shadow_rail_resource(gfx, prepared, screen_shadow)?,
            ];
            Some(rail.execute_rail(
                gfx,
                prepared,
                viewer,
                main_scene_view,
                main_scene,
                &pre_particle_scene.main_scene_target_sha256,
                &rail_resources,
            )?)
        } else {
            None
        };
        trees.execute_main_scene(
            gfx,
            prepared,
            viewer,
            tree_schedule,
            main_scene,
            &scene_cb0s,
            political
                .handoff
                .as_ref()
                .ok_or("XMAP11 execution has no retained XMAP05 provider")?,
            lighting_handoff,
            water
                .handoff
                .as_ref()
                .ok_or("XMAP11 execution has no retained XMAP07 provider")?,
            screen_shadow,
        )?;
        water.execute_complete_high_gfx_frame(
            gfx,
            prepared,
            viewer,
            main_scene,
            terrain,
            political,
            lighting,
            &scene_cb0_refs,
            &shadow_map,
        )?;
        thin_borders.execute_phase(
            gfx,
            prepared,
            viewer,
            main_scene,
            main_scene_view,
            screen_shadow,
            map_viewer::Xmap06BorderExecutionPhase::Late,
        )?;
        let thin_border_scene_receipt_sha256 = thin_borders.completed_scene_receipt_sha256()?;
        let water_scene_receipt_sha256 = water.completed_scene_receipt_sha256()?;
        let xmap15_present = matches!(
            prepared.xmap15_orders_state()?,
            map_viewer::Xmap15FoundationState::Present(_)
                | map_viewer::Xmap15FoundationState::AuthenticatedEmpty(_)
        );
        if xmap15_present != orders.executable_frame_present() {
            return Err("XMAP15 prepared/runtime availability join drifted".into());
        }
        let needs_arrow_resources = xmap15_present || (xmap16_present && supply_view_active);
        let (common_arrow_resources, point_lights_cb2) = if needs_arrow_resources {
            let mut common = vec![xmap16_shadow_supply_resource(gfx, prepared, screen_shadow)?];
            common.extend(terrain.xmap16_supply_resources(gfx, prepared)?);
            let (lighting_supply_resources, point_lights_cb2) =
                xmap16_lighting_supply_resources(lighting, gfx, prepared)?;
            common.extend(lighting_supply_resources);
            (Some(common), Some(point_lights_cb2))
        } else {
            (None, None)
        };
        let orders_resources = if xmap15_present {
            let mut resources = common_arrow_resources
                .as_ref()
                .ok_or("XMAP15 common external resources disappeared")?
                .clone();
            resources.extend(xmap15_water_supply_resources(gfx, prepared, water)?);
            Some(resources)
        } else {
            None
        };
        let _orders_execution = if xmap15_present {
            Some(
                orders.execute(
                    gfx,
                    prepared,
                    viewer,
                    main_scene_view,
                    main_scene,
                    &pre_particle_scene.main_scene_target_sha256,
                    orders_resources
                        .as_deref()
                        .ok_or("XMAP15 external resources disappeared")?,
                    point_lights_cb2
                        .as_ref()
                        .ok_or("XMAP15 PointLightsCB2 disappeared")?,
                )?,
            )
        } else {
            None
        };
        let supply_executions = if xmap16_present && supply_view_active {
            let mut supply_resources = common_arrow_resources
                .as_ref()
                .ok_or("XMAP16 common external resources disappeared")?
                .clone();
            supply_resources.push(xmap16_water_supply_resource(gfx, prepared, water)?);
            let receipts = rail.execute_supply(
                gfx,
                prepared,
                viewer,
                main_scene_view,
                main_scene,
                &supply_resources,
                point_lights_cb2
                    .as_ref()
                    .ok_or("XMAP16 PointLightsCB2 disappeared")?,
            )?;
            Some(receipts)
        } else {
            None
        };
        if xmap16_present {
            let rail_execution_sha256 = rail_execution
                .as_ref()
                .ok_or("XMAP16 railroad execution receipt disappeared")?
                .closure
                .closure_sha256
                .clone();
            let supply_execution_sha256s = supply_executions
                .as_ref()
                .map(|(normal, see_through)| {
                    vec![
                        normal.deterministic_sha256.clone(),
                        see_through.deterministic_sha256.clone(),
                    ]
                })
                .unwrap_or_default();
            rail.mark_core_execution_complete(rail_execution_sha256, supply_execution_sha256s)?;
        }
        let label_frame = prepared
            .labels
            .as_ref()
            .ok_or("XMAP17 product foundation has no prepared map-label frame")?;
        if label_frame.content_manifest_sha256 != prepared.envelope.content_manifest_sha256
            || label_frame.product_modset_sha256 != prepared.envelope.product_modset_sha256
            || label_frame.effective_vfs_manifest_sha256
                != prepared.envelope.effective_vfs_manifest_sha256
        {
            return Err(
                "XMAP17 label content authority differs from its foundation envelope".into(),
            );
        }
        let replace_label_renderer = self
            .map_label_program_sha256
            .as_deref()
            .is_none_or(|sha256| sha256 != label_frame.program.deterministic_sha256);
        if replace_label_renderer {
            self.map_label_renderer = Some(crate::map_labels::MapLabelRenderer::new_product(
                gfx,
                resolver,
                label_frame,
            )?);
            self.map_label_program_sha256 = Some(label_frame.program.deterministic_sha256.clone());
            self.map_label_alpha = Some(map_viewer::MapNameAlphaState::new(0.0, 0.0)?);
        }
        let (near_selector, far_selector, update_daily) =
            prepared.map_modes.xmap17_text_selection()?;
        let label_schedule = map_viewer::AssignedStageSchedule::from_topology(
            &map_viewer::product_foundation_assigned_stage_topology()?,
        )
        .map_err(|error| format!("XMAP17 product schedule: {error}"))?;
        let label_stage = label_schedule
            .require(
                map_viewer::XMAP17_EXECUTE_STAGE,
                map_viewer::XMAP17_EXECUTE_OPERATION,
            )?
            .clone();
        let label_ready = label_frame.admit_final_image_dependencies(
            map_viewer::MapLabelFinalImageDependencies {
                scene_cb0_sha256: main_scene_view.payload_sha256().to_owned(),
                final_image_target_sha256: main_scene.resource_identity_sha256().to_owned(),
                xmap10_operation_order_sha256: label_schedule.schedule_sha256.clone(),
                near_selector,
                far_selector,
                update_daily,
            },
            self.map_label_alpha
                .as_ref()
                .copied()
                .ok_or("XMAP17 temporal alpha state disappeared")?,
            delta_seconds,
        )?;
        let next_label_alpha = label_ready.next_alpha_state()?;
        let label_identity = map_viewer::Xmap17ExecutionIdentity {
            hour_index: prepared.envelope.hour_index,
            content_manifest_sha256: prepared.envelope.content_manifest_sha256.clone(),
            foundation_content_operation_sha256: prepared
                .envelope
                .foundation_content_operation_sha256
                .clone(),
            viewer_state_sha256: snapshot::sha256::Sha256::hex(&serde_json::to_vec(viewer)?),
            main_scene_target_sha256: main_scene.resource_identity_sha256().to_owned(),
            map_copy_receipt_set_sha256: main_scene_view.map_copy_receipt_set_sha256().to_owned(),
        };
        let label_draw = self
            .map_label_renderer
            .as_ref()
            .ok_or("XMAP17 renderer disappeared")?
            .draw_prepared_product(
                gfx,
                label_frame,
                &label_ready,
                main_scene_view,
                main_scene,
                label_identity,
                label_stage,
            )?;
        self.map_label_alpha = Some(next_label_alpha);
        self.map_label_draw = Some(label_draw);
        let terrain_draw_receipt_sha256 = self
            .terrain_draw
            .as_ref()
            .ok_or("XMAP04 terrain draw receipt disappeared")?
            .deterministic_sha256
            .clone();
        let viewer_state_sha256 = water.viewer_state_sha256()?.to_owned();
        let xmap13_identities =
            map_viewer::prepare_xmap13_effect_identity(resolver, content_admission, world_frame)?;
        let weather = prepared
            .weather
            .as_ref()
            .ok_or("XMAP20 requires the current XMAP19 weather identity provider")?;
        weather.validate()?;
        let effect_frame = world_frame
            .xmap20_effects()
            .ok_or("XMAP20 requires the unconditional current-hour effect lifecycle carrier")?;
        let production_camera = camera.production_camera()?;
        let projection_bytes = production_camera
            .view_projection_matrix
            .iter()
            .flatten()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        if projection_bytes.len() != 64 {
            return Err("XMAP03 projection CB1 did not serialize to exactly 64 bytes".into());
        }
        let projection_cb1 = replay_core::ConstantBufferBinding {
            stage_mask_u8: 0b1,
            slot: 1,
            byte_offset: 0,
            byte_count: 64,
            sha256: snapshot::sha256::Sha256::hex(&projection_bytes),
            raw_bytes: projection_bytes,
            semantic_layout: "XMAP03.ViewProjectionMatrix.float4x4.row-major".into(),
        };
        let weather_ids = weather
            .ordered_producer_instance_ids()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let lighting_execution = lighting_handoff
            .receipt()
            .light_depth_execution
            .as_ref()
            .ok_or("XMAP20 XMAP09 provider has no completed light/depth execution receipt")?;
        let dependencies = replay_core::Xmap20DependencyReceipts {
            schema: replay_core::XMAP20_PROVIDER_RECEIPTS_SCHEMA.into(),
            hour_index: prepared.envelope.hour_index,
            content_manifest_sha256: prepared.envelope.content_manifest_sha256.clone(),
            product_modset_sha256: prepared.envelope.product_modset_sha256.clone(),
            effective_vfs_manifest_sha256: prepared.envelope.effective_vfs_manifest_sha256.clone(),
            camera: replay_core::Xmap20DependencyReceipt {
                owner_packet: replay_core::XMAP20_CAMERA_OWNER.into(),
                provider_schema: map_viewer::VIEWER_MAP_CAMERA_SCHEMA.into(),
                payload_sha256: camera.deterministic_sha256.clone(),
                descriptor_sha256: projection_cb1.sha256.clone(),
                execution_sha256: camera.deterministic_sha256.clone(),
            },
            lighting: replay_core::Xmap20DependencyReceipt {
                owner_packet: replay_core::XMAP20_LIGHTING_OWNER.into(),
                provider_schema: map_viewer::XMAP09_PREPARED_SCHEMA.into(),
                payload_sha256: lighting_frame.deterministic_sha256.clone(),
                descriptor_sha256: lighting_handoff.receipt().scene_cb0_patch_sha256.clone(),
                execution_sha256: lighting_execution.deterministic_sha256.clone(),
            },
            scene_compositor_pre_particles: replay_core::Xmap20DependencyReceipt {
                owner_packet: replay_core::XMAP20_PRE_PARTICLE_COMPOSITOR_OWNER.into(),
                provider_schema: map_viewer::XMAP10_PRE_PARTICLE_SCENE_SCHEMA.into(),
                payload_sha256: pre_particle_scene.pre_particle_admission_sha256.clone(),
                descriptor_sha256: pre_particle_scene.main_scene_target_sha256.clone(),
                execution_sha256: pre_particle_scene.deterministic_sha256.clone(),
            },
            entity_identity: xmap13_identities.receipt().xmap20_dependency_receipt()?,
            weather_identity: replay_core::Xmap20DependencyReceipt {
                owner_packet: replay_core::XMAP20_WEATHER_IDENTITY_OWNER.into(),
                provider_schema: map_viewer::XMAP19_WEATHER_SCHEMA.into(),
                payload_sha256: weather.deterministic_sha256.clone(),
                descriptor_sha256: snapshot::sha256::Sha256::hex(&serde_json::to_vec(
                    &weather_ids,
                )?),
                execution_sha256: weather.deterministic_sha256.clone(),
            },
            ordered_entity_semantic_source_ids: xmap13_identities
                .ordered_semantic_source_ids()
                .to_vec(),
            ordered_weather_semantic_source_ids: weather_ids,
        };
        dependencies
            .validate()
            .map_err(|error| format!("XMAP20 provider receipts: {error}"))?;
        let trigger_provider = map_viewer::Xmap20ProductionTriggerProvider {
            entity_identities: &xmap13_identities,
            weather,
        };
        let prepared_effects = map_viewer::prepare_xmap20_effects(
            resolver,
            effect_frame,
            &dependencies,
            &trigger_provider,
        )?;
        let xmap20_topology = map_viewer::product_foundation_assigned_stage_topology()?;
        let xmap20_schedule = map_viewer::AssignedStageSchedule::from_topology(&xmap20_topology)
            .map_err(|error| format!("XMAP20 product schedule: {error}"))?;
        let xmap20_stage = xmap20_schedule
            .require(
                map_viewer::XMAP20_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP20_ASSIGNED_OPERATION_NAME,
            )?
            .clone();
        let shared = crate::xmap20_effects::Xmap20SharedExecutionIdentities {
            content_manifest_sha256: prepared.envelope.content_manifest_sha256.clone(),
            foundation_content_operation_sha256: prepared
                .envelope
                .foundation_content_operation_sha256
                .clone(),
            viewer_state_sha256: viewer_state_sha256.clone(),
            main_scene_target_sha256: replay_core::canonical_json_sha256(main_scene.desc())?,
            main_scene_resource_identity_sha256: main_scene.resource_identity_sha256().to_owned(),
            main_scene_allocation_generation: main_scene.allocation_generation(),
            pre_particle_scene_receipt_sha256: pre_particle_scene.deterministic_sha256.clone(),
            map_copy_receipt_set_sha256: main_scene_view.map_copy_receipt_set_sha256().to_owned(),
        };
        let xmap20_status = if prepared_effects.ordered_particles.is_empty() {
            let scene_receipt_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(
                "hoi4.xmap20.authenticated-empty-scene.v1",
                &prepared_effects.deterministic_sha256,
                &prepared_effects.admission.complete_enumeration_sha256,
                &shared.main_scene_resource_identity_sha256,
                shared.main_scene_allocation_generation,
                &shared.pre_particle_scene_receipt_sha256,
            ))?);
            ProductXmap20ExecutionStatus::AuthenticatedEmpty {
                prepared_frame_sha256: prepared_effects.deterministic_sha256.clone(),
                complete_enumeration_sha256: prepared_effects
                    .admission
                    .complete_enumeration_sha256
                    .clone(),
                main_scene_resource_identity_sha256: shared
                    .main_scene_resource_identity_sha256
                    .clone(),
                main_scene_allocation_generation: shared.main_scene_allocation_generation,
                pre_particle_scene_receipt_sha256: shared.pre_particle_scene_receipt_sha256.clone(),
                scene_receipt_sha256,
            }
        } else {
            ProductXmap20ExecutionStatus::Present(crate::xmap20_effects::execute_xmap20_effects(
                gfx,
                resolver,
                &prepared_effects,
                &dependencies,
                &xmap20_stage,
                &shared,
                &pre_particle_scene,
                &projection_cb1,
                main_scene,
            )?)
        };
        xmap20_status.validate()?;
        self.xmap20_effects = Some(xmap20_status);
        let xmap20_scene_receipt_sha256 = self
            .xmap20_effects
            .as_ref()
            .ok_or("XMAP20 execution status disappeared")?
            .scene_receipt_sha256()
            .to_owned();
        let xmap21_topology = map_viewer::product_foundation_assigned_stage_topology()?;
        let xmap21_schedule = map_viewer::AssignedStageSchedule::from_topology(&xmap21_topology)
            .map_err(|error| format!("XMAP21 product schedule: {error}"))?;
        let furniture_state_stage = xmap21_schedule
            .require("map_mode", "map_mode.furniture_state")?
            .clone();
        let furniture_prepare_stage = xmap21_schedule
            .require(
                map_viewer::XMAP21_PREPARE_STAGE_NAME,
                map_viewer::XMAP21_PREPARE_OPERATION_NAME,
            )?
            .clone();
        let furniture_execute_stage = xmap21_schedule
            .require(
                map_viewer::XMAP21_EXECUTE_STAGE_NAME,
                map_viewer::XMAP21_EXECUTE_OPERATION_NAME,
            )?
            .clone();
        if furniture_state_stage.assigned_ordinal >= furniture_prepare_stage.assigned_ordinal {
            return Err(
                "XMAP21 prepare is not ordered after the XMAP18 furniture-state handoff".into(),
            );
        }
        let (furniture_mode_id, furniture_viewer_token) = match prepared.map_modes.composition {
            map_viewer::MapModeComposition::ResourceFurniture => (
                map_viewer::XMAP21_RESOURCE_MODE_ID.to_owned(),
                map_viewer::XMAP21_RESOURCE_VIEWER_TOKEN.to_owned(),
            ),
            map_viewer::MapModeComposition::RaidFurniture => (
                map_viewer::XMAP21_RAID_MODE_ID.to_owned(),
                map_viewer::XMAP21_RAID_VIEWER_TOKEN.to_owned(),
            ),
            composition => (
                composition.token().to_owned(),
                prepared.map_modes.viewer_selection.clone(),
            ),
        };
        let selected_country = viewer.observer_country_id.as_deref();
        let selected_country_db_id = match selected_country {
            None => None,
            Some(tag) => Some(
                world_frame
                    .d18_country_roster()
                    .entries
                    .iter()
                    .find(|entry| entry.tag == tag)
                    .ok_or("XMAP21 selected country is absent from the complete D18 roster")?
                    .tag_db_id,
            ),
        };
        let xmap21_request = map_viewer::MapFurniturePrepareRequest::seal(
            map_viewer::MapModeFurnitureSelection {
                update_index: u64::try_from(prepared.envelope.hour_index)
                    .map_err(|_| "XMAP21 cannot bind a negative replay hour")?,
                selection_epoch: viewer.map_mode_selection_epoch,
                mode_id: furniture_mode_id,
                viewer_token: furniture_viewer_token,
            },
            selected_country.map(str::to_owned),
            selected_country_db_id,
            map_viewer::MapFurnitureSharedIdentities {
                content_manifest_sha256: prepared.envelope.content_manifest_sha256.clone(),
                foundation_content_operation_sha256: prepared
                    .envelope
                    .foundation_content_operation_sha256
                    .clone(),
                viewer_state_sha256: viewer_state_sha256.clone(),
                main_scene_target_sha256: main_scene.resource_identity_sha256().to_owned(),
                map_copy_receipt_set_sha256: main_scene_view
                    .map_copy_receipt_set_sha256()
                    .to_owned(),
            },
            furniture_prepare_stage,
            furniture_execute_stage,
        )?;
        let production_camera = camera.production_camera()?;
        let xmap21_packet = furniture.prepare_packet(
            world_frame,
            resolver,
            localisation_language,
            &xmap21_request,
            &production_camera,
            main_scene.desc().color_width as f32,
            main_scene.desc().color_height as f32,
        )?;
        self.xmap21_packet = Some(xmap21_packet);
        let upstream_map_scene_receipt_sha256 =
            snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(
                "hoi4.replay.post-particle-map-scene.v1",
                terrain_draw_receipt_sha256,
                thin_border_scene_receipt_sha256,
                water_scene_receipt_sha256,
                rivers_straits_scene_receipt_sha256,
                rail_execution
                    .as_ref()
                    .map(|receipt| receipt.closure.closure_sha256.as_str()),
                supply_executions
                    .as_ref()
                    .map(|(normal, _)| normal.deterministic_sha256.as_str()),
                supply_executions
                    .as_ref()
                    .map(|(_, see_through)| see_through.deterministic_sha256.as_str()),
                self.map_label_draw
                    .as_ref()
                    .map(|receipt| receipt.packet_sha256.as_str()),
                xmap20_scene_receipt_sha256,
            ))?);
        if self.postfx_schedule.is_none() {
            let topology = map_viewer::xmap10_postfx_assigned_stage_topology()?;
            self.postfx_schedule = Some(
                map_viewer::AssignedStageSchedule::from_topology(&topology)
                    .map_err(|error| format!("XMAP10 post-FX schedule: {error}"))?,
            );
        }
        if replace_main_scene || self.postfx.is_none() {
            self.postfx = Some(
                map_viewer::NativeXmap10PostFxExecutor::create_on_main_scene(
                    gfx.device.as_raw(),
                    gfx.context.as_raw(),
                    resolver,
                    &prepared_xmap10,
                    main_scene,
                )?,
            );
        }
        let postfx = self
            .postfx
            .as_mut()
            .ok_or("XMAP10 native post-FX executor disappeared")?;
        postfx.prepare_frame(resolver, &prepared_xmap10)?;
        let swapchain_target = prepared_xmap10
            .targets
            .ordered_targets
            .iter()
            .find(|target| target.role == map_viewer::Xmap10TargetRole::Swapchain)
            .ok_or("XMAP10 target plan omitted the application target")?;
        let final_target = map_viewer::Xmap10AttachedFinalTarget {
            width: gfx.width,
            height: gfx.height,
            dxgi_format: gfx.target_format(),
            sample_count: 1,
            descriptor_sha256: swapchain_target.descriptor_sha256.clone(),
            target_identity_sha256: snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(
                &serde_json::to_vec(&(
                    "replay-app.presentation-target.v1",
                    gfx.width,
                    gfx.height,
                    gfx.target_format(),
                    gfx.target_allocation_generation(),
                ))?,
            )),
            resource_identity_sha256: gfx.target_resource_identity_sha256()?.to_owned(),
            allocation_generation: gfx.target_allocation_generation(),
        };
        let postfx_input = map_viewer::Xmap10PostFxExecutionInput {
            content_manifest_sha256: prepared_xmap10.content.content_manifest_sha256.clone(),
            foundation_content_operation_sha256: prepared
                .envelope
                .foundation_content_operation_sha256
                .clone(),
            viewer_state_sha256,
            main_scene_target_sha256: pre_particle_scene.main_scene_target_sha256.clone(),
            map_copy_receipt_set_sha256: main_scene_view.map_copy_receipt_set_sha256().to_owned(),
            upstream_map_scene_receipt_sha256,
            assigned_stage_schedule: self
                .postfx_schedule
                .as_ref()
                .ok_or("XMAP10 post-FX schedule disappeared")?
                .clone(),
        };
        let temporal_policy = match &temporal_receipt.source {
            map_viewer::Xmap10TemporalAdaptationSource::Continue {
                prior_frame_sequence,
                ..
            } => map_viewer::Xmap10TemporalPolicy::Continue {
                prior_frame_sequence: *prior_frame_sequence,
            },
            map_viewer::Xmap10TemporalAdaptationSource::EngineResourceReset {
                reason,
                main_scene_resource_identity_sha256,
                allocation_generation,
            } => map_viewer::Xmap10TemporalPolicy::EngineResourceReset {
                reason: *reason,
                main_scene_resource_identity_sha256: main_scene_resource_identity_sha256.clone(),
                allocation_generation: *allocation_generation,
            },
        };
        let temporal_execution = map_viewer::Xmap10TemporalExecutionInput {
            frame_sequence: temporal_receipt.frame_sequence,
            delta_seconds: if matches!(
                &temporal_receipt.source,
                map_viewer::Xmap10TemporalAdaptationSource::EngineResourceReset { .. }
            ) {
                0.0
            } else {
                delta_seconds
            },
            scalar_source_receipt_sha256: prepared_xmap10.grade.deterministic_sha256.clone(),
            policy: temporal_policy,
        };
        let reduction =
            map_viewer::Xmap10LuminanceReductionPlan::from_target_plan(&prepared_xmap10.targets)?;
        self.completed_postfx = Some(postfx.execute(
            &prepared_xmap10,
            &pre_particle_scene,
            &postfx_input,
            main_scene_view,
            gfx.rtv()?.as_raw(),
            &final_target,
            &reduction,
            &temporal_execution,
        )?);
        self.temporal_receipt = Some(temporal_receipt);
        self.temporal_hour_index = Some(prepared.envelope.hour_index);
        let providers = self.current_providers()?;
        if providers.main_scene.device_identity() != gfx.device.as_raw() as usize
            || providers.main_scene_cb0.render_view_mask() != 1
            || providers.shadow_map.device_identity() != gfx.device.as_raw() as usize
            || providers
                .shadow_receipt
                .binding
                .consumer_camera_frame_sha256
                != prepared.terrain.camera_frame_sha256
        {
            return Err("XMAP10 downstream provider bundle failed current-frame admission".into());
        }
        Ok(&water.execution)
    }

    unsafe fn execute_map_icon_buckets<C, H, FC, FH>(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        furniture: &mut FurnitureRenderPass,
        counter_execution_sha256: &str,
        execute_combat_buckets: FC,
        execute_hub_bucket: FH,
    ) -> Result<(C, H)>
    where
        FC: FnOnce() -> Result<(C, String)>,
        FH: FnOnce() -> Result<(H, String)>,
    {
        if self.completed_postfx.is_none() || self.xmap21_execution.is_some() {
            return Err("XMAP22 map-icon buckets require one completed post-FX frame".into());
        }
        let packet = self
            .xmap21_packet
            .as_ref()
            .ok_or("XMAP22 map-icon buckets have no prepared XMAP21 packet")?;
        let final_rtv = gfx.rtv()?;
        let target = crate::furniture_render::AuthenticatedMapSceneTarget {
            kind: crate::furniture_render::MapSceneTargetKind::FinalImage,
            target_identity_sha256: gfx.target_resource_identity_sha256()?.to_owned(),
            render_target_view: final_rtv.as_raw(),
            width: gfx.width,
            height: gfx.height,
            dxgi_format: gfx.target_format(),
            sample_count: 1,
            sample_quality: 0,
        };
        let (receipt, combat_outcome, hub_outcome) = furniture
            .execute_prepared_packet_interleaved(
                gfx,
                resolver,
                &target,
                packet,
                counter_execution_sha256,
                execute_combat_buckets,
                execute_hub_bucket,
            )?;
        receipt.validate()?;
        self.xmap21_execution = Some(receipt);
        Ok((combat_outcome, hub_outcome))
    }
}

fn xmap16_lighting_supply_resources(
    lighting_runtime: &ProductLightingHandoffRuntime,
    gfx: &Gfx,
    prepared: &map_viewer::PreparedFoundationFrame,
) -> Result<(
    Vec<map_viewer::NativeXmap16ExternalResourceView>,
    map_viewer::NativeXmap16ExternalConstantBufferView,
)> {
    let lighting = prepared
        .lighting
        .as_ref()
        .ok_or("XMAP09 lighting is unavailable at XMAP16 supply execution")?;
    lighting.validate()?;
    let handoff = lighting_runtime
        .handoff
        .as_ref()
        .ok_or("XMAP09 resources are absent at XMAP16 supply execution")?;
    let receipt = handoff.receipt();
    if receipt.prepared_frame_sha256 != lighting.deterministic_sha256
        || receipt.camera_frame_sha256 != lighting.camera_frame_sha256
        || receipt.replay_frame_identity_sha256 != lighting.replay_frame_identity_sha256
        || receipt.replay_frame_identity_sha256 != prepared.terrain.replay_frame_identity_sha256
    {
        return Err("XMAP09 resources are not from the current replay/camera frame".into());
    }

    let resources = [
        (handoff.light_data_map_view(), &lighting.light_data_map, 9),
        (
            handoff.light_index_map_view(),
            &lighting.light_index_map,
            10,
        ),
    ]
    .into_iter()
    .map(|(resource, expected, shader_register)| {
        if resource.frame_identity_sha256() != lighting.camera_frame_sha256
            || resource.replay_frame_identity_sha256() != lighting.replay_frame_identity_sha256
            || resource.binding.stable_resource_id != expected.stable_resource_id
            || resource.binding.resource_sha256 != expected.payload_sha256
            || resource.binding.descriptor_sha256 != expected.descriptor_sha256
            || !receipt
                .point_light_bindings
                .iter()
                .any(|binding| binding == &resource.binding)
        {
            return Err(
                "XMAP09 supply resource does not match the current prepared lighting frame".into(),
            );
        }
        unsafe {
            require_current_srv_device(
                gfx,
                resource.as_raw(),
                &format!("XMAP09 {}", resource.binding.symbol),
            )?;
            map_viewer::NativeXmap16ExternalResourceView::from_provider(
                map_viewer::Xmap16SupplyArrowResourceIdentity {
                    symbol: resource.binding.symbol.clone(),
                    shader_register,
                    owner_packet: resource.binding.owner_packet.clone(),
                    payload_sha256: resource.binding.resource_sha256.clone(),
                    descriptor_sha256: resource.binding.descriptor_sha256.clone(),
                },
                resource.as_raw(),
            )
        }
    })
    .collect::<Result<Vec<_>>>()?;

    let cb2 = handoff.point_lights_cb2_view();
    if cb2.frame_identity_sha256() != lighting.camera_frame_sha256
        || cb2.replay_frame_identity_sha256() != lighting.replay_frame_identity_sha256
        || cb2.binding != receipt.point_lights_cb2
    {
        return Err("XMAP09 PointLightsCB2 is not from the current replay/camera frame".into());
    }
    let raw_cb2 = cb2.as_raw();
    let binding = cb2.binding.clone();
    let cb2 = unsafe {
        require_current_buffer_device(gfx, raw_cb2, "XMAP09 PointLightsCB2")?;
        map_viewer::NativeXmap16ExternalConstantBufferView::from_provider(
            binding.symbol,
            binding.shader_register,
            binding.owner_packet,
            binding.byte_len,
            binding.payload_sha256,
            binding.descriptor_sha256,
            raw_cb2,
        )?
    };
    Ok((resources, cb2))
}

fn xmap16_water_rail_resource(
    gfx: &Gfx,
    prepared: &map_viewer::PreparedFoundationFrame,
    water: &ProductWaterHandoffRuntime,
) -> Result<map_viewer::NativeXmap16RailExternalResourceView> {
    let handoff = water
        .handoff
        .as_ref()
        .ok_or("XMAP07 resources are absent at XMAP16 rail execution")?;
    let receipt = handoff.receipt();
    receipt.validate()?;
    if receipt.water_frame_sha256 != prepared.water.deterministic_sha256
        || receipt.replay_frame_identity_sha256 != prepared.terrain.replay_frame_identity_sha256
    {
        return Err("XMAP07 SnowMudTexture is not from the current replay frame".into());
    }
    let view = handoff.resource_view("SnowMudTexture")?;
    if view.device_identity != gfx.device.as_raw() as usize
        || view.replay_frame_identity_sha256 != prepared.water.replay_frame_identity_sha256
        || !receipt.exposed_bindings.contains(&view.binding)
    {
        return Err("XMAP07 SnowMudTexture failed the current-frame provider join".into());
    }
    let view = view.at_shader_register(8);
    unsafe {
        require_current_srv_device(gfx, view.as_raw(), "XMAP07 SnowMudTexture")?;
        map_viewer::NativeXmap16RailExternalResourceView::from_provider(
            map_viewer::Xmap16RailExternalResourceIdentity {
                symbol: view.binding.symbol.clone(),
                shader_register: 8,
                owner_packet: view.binding.owner_packet.clone(),
                payload_sha256: view.binding.resource_sha256.clone(),
                descriptor_sha256: view.binding.descriptor_sha256.clone(),
            },
            view.as_raw(),
        )
    }
}

fn xmap16_water_supply_resource(
    gfx: &Gfx,
    prepared: &map_viewer::PreparedFoundationFrame,
    water: &ProductWaterHandoffRuntime,
) -> Result<map_viewer::NativeXmap16ExternalResourceView> {
    let handoff = water
        .handoff
        .as_ref()
        .ok_or("XMAP07 resources are absent at XMAP16 supply execution")?;
    let receipt = handoff.receipt();
    receipt.validate()?;
    if receipt.water_frame_sha256 != prepared.water.deterministic_sha256
        || receipt.replay_frame_identity_sha256 != prepared.terrain.replay_frame_identity_sha256
    {
        return Err("XMAP07 LeanTexture2 is not from the current replay frame".into());
    }
    let view = handoff.resource_view("LeanTexture2")?;
    if view.device_identity != gfx.device.as_raw() as usize
        || view.replay_frame_identity_sha256 != prepared.water.replay_frame_identity_sha256
        || !receipt.exposed_bindings.contains(&view.binding)
    {
        return Err("XMAP07 LeanTexture2 failed the current-frame provider join".into());
    }
    let view = view.at_shader_register(8);
    unsafe {
        require_current_srv_device(gfx, view.as_raw(), "XMAP07 LeanTexture2")?;
        map_viewer::NativeXmap16ExternalResourceView::from_provider(
            map_viewer::Xmap16SupplyArrowResourceIdentity {
                symbol: view.binding.symbol.clone(),
                shader_register: 8,
                owner_packet: view.binding.owner_packet.clone(),
                payload_sha256: view.binding.resource_sha256.clone(),
                descriptor_sha256: view.binding.descriptor_sha256.clone(),
            },
            view.as_raw(),
        )
    }
}

fn xmap16_shadow_rail_resource(
    gfx: &Gfx,
    prepared: &map_viewer::PreparedFoundationFrame,
    screen_shadow: &map_viewer::NativeXmap10ScreenShadowHandoff,
) -> Result<map_viewer::NativeXmap16RailExternalResourceView> {
    let view = screen_shadow.shadow_map_view()?.at_shader_register(15);
    if view.device_identity() != gfx.device.as_raw() as usize
        || view.frame_identity_sha256() != prepared.terrain.camera_frame_sha256
        || view.replay_frame_identity_sha256() != prepared.terrain.replay_frame_identity_sha256
    {
        return Err("XMAP10 ShadowMap failed the current rail-frame provider join".into());
    }
    let binding = view.binding();
    unsafe {
        require_current_srv_device(gfx, view.as_raw(), "XMAP10 ShadowMap")?;
        map_viewer::NativeXmap16RailExternalResourceView::from_provider(
            map_viewer::Xmap16RailExternalResourceIdentity {
                symbol: binding.symbol.clone(),
                shader_register: 15,
                owner_packet: binding.owner_packet.clone(),
                payload_sha256: binding.resource_sha256.clone(),
                descriptor_sha256: binding.descriptor_sha256.clone(),
            },
            view.as_raw(),
        )
    }
}

fn xmap16_shadow_supply_resource(
    gfx: &Gfx,
    prepared: &map_viewer::PreparedFoundationFrame,
    screen_shadow: &map_viewer::NativeXmap10ScreenShadowHandoff,
) -> Result<map_viewer::NativeXmap16ExternalResourceView> {
    let view = screen_shadow.shadow_map_view()?.at_shader_register(2);
    if view.device_identity() != gfx.device.as_raw() as usize
        || view.frame_identity_sha256() != prepared.terrain.camera_frame_sha256
        || view.replay_frame_identity_sha256() != prepared.terrain.replay_frame_identity_sha256
    {
        return Err("XMAP10 ShadowMap failed the current supply-frame provider join".into());
    }
    let binding = view.binding();
    unsafe {
        require_current_srv_device(gfx, view.as_raw(), "XMAP10 ShadowMap")?;
        map_viewer::NativeXmap16ExternalResourceView::from_provider(
            map_viewer::Xmap16SupplyArrowResourceIdentity {
                symbol: binding.symbol.clone(),
                shader_register: 2,
                owner_packet: binding.owner_packet.clone(),
                payload_sha256: binding.resource_sha256.clone(),
                descriptor_sha256: binding.descriptor_sha256.clone(),
            },
            view.as_raw(),
        )
    }
}

fn xmap15_water_supply_resources(
    gfx: &Gfx,
    prepared: &map_viewer::PreparedFoundationFrame,
    water: &ProductWaterHandoffRuntime,
) -> Result<Vec<map_viewer::NativeXmap16ExternalResourceView>> {
    let handoff = water
        .handoff
        .as_ref()
        .ok_or("XMAP07 resources are absent at XMAP15 orders execution")?;
    let receipt = handoff.receipt();
    receipt.validate()?;
    if receipt.water_frame_sha256 != prepared.water.deterministic_sha256
        || receipt.replay_frame_identity_sha256 != prepared.terrain.replay_frame_identity_sha256
    {
        return Err("XMAP07 LEAN resources are not from the current replay frame".into());
    }
    let mut resources = Vec::with_capacity(2);
    for (symbol, register) in [("LeanTexture1", 7u32), ("LeanTexture2", 8u32)] {
        let view = handoff.resource_view(symbol)?;
        if view.device_identity != gfx.device.as_raw() as usize
            || view.replay_frame_identity_sha256 != prepared.water.replay_frame_identity_sha256
            || !receipt.exposed_bindings.contains(&view.binding)
        {
            return Err(format!("XMAP07 {symbol} failed the current-frame provider join").into());
        }
        let view = view.at_shader_register(register);
        unsafe {
            require_current_srv_device(gfx, view.as_raw(), &format!("XMAP07 {symbol}"))?;
            resources.push(map_viewer::NativeXmap16ExternalResourceView::from_provider(
                map_viewer::Xmap16SupplyArrowResourceIdentity {
                    symbol: view.binding.symbol.clone(),
                    shader_register: register,
                    owner_packet: view.binding.owner_packet.clone(),
                    payload_sha256: view.binding.resource_sha256.clone(),
                    descriptor_sha256: view.binding.descriptor_sha256.clone(),
                },
                view.as_raw(),
            )?);
        }
    }
    Ok(resources)
}

impl ProductOrdersHandoffRuntime {
    fn retain_frame(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        prepared: &map_viewer::PreparedFoundationFrame,
    ) -> Result<()> {
        let frame = match prepared.xmap15_orders_state()? {
            map_viewer::Xmap15FoundationState::Present(frame)
            | map_viewer::Xmap15FoundationState::AuthenticatedEmpty(frame) => frame,
            map_viewer::Xmap15FoundationState::Unavailable(gap) => {
                self.native = None;
                self.lifecycle_update = None;
                self.execution_receipt = None;
                self.unavailable_reason = Some(gap.reason.clone());
                return Ok(());
            }
        };
        self.unavailable_reason = None;
        self.execution_receipt = None;
        if self.native.as_ref().is_some_and(|native| {
            native.prepared_frame_sha256() == frame.portable.deterministic_sha256
        }) && self.lifecycle_update.as_ref().is_some_and(|update| {
            update.hour_index == frame.portable.hour_index
                && update.observation_sequence == frame.portable.observation_sequence
        }) {
            return Ok(());
        }
        let update = self.lifecycle.apply(frame)?;
        let native = unsafe {
            map_viewer::NativeXmap15Handoff::create_on_device(gfx.device.as_raw(), resolver, frame)?
        };
        self.native = Some(native);
        self.lifecycle_update = Some(update);
        Ok(())
    }

    fn executable_frame_present(&self) -> bool {
        self.unavailable_reason.is_none()
            && self.native.is_some()
            && self.lifecycle_update.is_some()
    }

    unsafe fn execute<S>(
        &mut self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        scene_cb0: &S,
        target: &dyn map_viewer::NativeXmap16MainSceneTargetView,
        target_sha256: &str,
        external_resources: &[map_viewer::NativeXmap16ExternalResourceView],
        point_lights_cb2: &map_viewer::NativeXmap16ExternalConstantBufferView,
    ) -> Result<&map_viewer::Xmap15NativeExecutionReceipt>
    where
        S: map_viewer::NativeXmap16SceneConstantBufferView,
    {
        let frame = prepared.require_xmap15_orders()?;
        let camera = viewer
            .camera
            .as_ref()
            .ok_or("XMAP15 execution has no viewer-owned camera")?
            .production_camera()?;
        let lifecycle = self
            .lifecycle_update
            .as_ref()
            .ok_or("XMAP15 execution has no lifecycle update")?;
        let native = self
            .native
            .as_ref()
            .ok_or("XMAP15 execution has no retained native handoff")?;
        let application_elapsed_seconds =
            map_viewer::NativeXmap16SceneConstantBufferView::application_elapsed_seconds(scene_cb0);
        let receipt = native.draw(map_viewer::NativeXmap15DrawInputs {
            context: gfx.context.as_raw(),
            target,
            target_sha256,
            prepared_frame: frame,
            lifecycle_update_sha256: &lifecycle.deterministic_sha256,
            scene_cb0,
            point_lights_cb2,
            external_resources,
            view_projection_matrix: camera.view_projection_matrix,
            application_elapsed_seconds,
        })?;
        self.execution_receipt = Some(receipt);
        Ok(self
            .execution_receipt
            .as_ref()
            .expect("XMAP15 execution receipt was just stored"))
    }

    fn execution_complete(&self) -> bool {
        self.execution_receipt
            .as_ref()
            .is_some_and(|receipt| receipt.validate().is_ok())
    }

    fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }
}

impl ProductRailHandoffRuntime {
    fn retain_frame(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        prepared: &map_viewer::PreparedFoundationFrame,
    ) -> Result<Option<&map_viewer::Xmap16RailHandoffReceipt>> {
        let rail = match prepared.xmap16_rail_supply_state()? {
            map_viewer::Xmap16FoundationState::Present(rail) => rail,
            map_viewer::Xmap16FoundationState::Unavailable(gap) => {
                self.rail = None;
                self.supply = None;
                self.hub = None;
                self.hub_packet = None;
                self.unavailable_reason = Some(gap.reason.clone());
                self.core_execution_complete = false;
                self.rail_execution_sha256 = None;
                self.supply_execution_sha256s.clear();
                return Ok(None);
            }
        };
        self.unavailable_reason = None;
        self.core_execution_complete = false;
        self.rail_execution_sha256 = None;
        self.supply_execution_sha256s.clear();
        if self.rail.as_ref().is_some_and(|handoff| {
            handoff.receipt().program_sha256 != rail.program.deterministic_sha256
                || handoff.receipt().rail_texture_sha256 != rail.rail.railroad_texture.sha256
        }) {
            self.rail = None;
        }
        if self.rail.is_none() {
            self.rail = Some(unsafe {
                map_viewer::NativeXmap16RailHandoff::create_on_device(
                    gfx.device.as_raw(),
                    resolver,
                    rail,
                )?
            });
        } else {
            unsafe {
                self.rail
                    .as_mut()
                    .ok_or("XMAP16 rail handoff disappeared")?
                    .update_frame(gfx.device.as_raw(), rail)?;
            }
        }
        if self.supply.as_ref().is_some_and(|handoff| {
            handoff.receipt().program_sha256 != rail.supply_program.deterministic_sha256
        }) {
            self.supply = None;
        }
        if self.supply.is_none() {
            self.supply = Some(unsafe {
                map_viewer::NativeXmap16SupplyArrowHandoff::create_on_device(
                    gfx.device.as_raw(),
                    &rail.supply_program,
                )?
            });
        }
        unsafe {
            let supply = self
                .supply
                .as_mut()
                .ok_or("XMAP16 supply-arrow handoff disappeared")?;
            supply.upload_geometry(gfx.device.as_raw(), &rail.supply_mesh)?;
            supply.upload_definition_textures(
                gfx.device.as_raw(),
                resolver,
                &rail.supply_definitions,
            )?;
        }
        Ok(Some(
            self.rail
                .as_ref()
                .ok_or("XMAP16 rail handoff was not retained")?
                .receipt(),
        ))
    }

    fn executable_frame_present(&self) -> bool {
        self.unavailable_reason.is_none() && self.rail.is_some() && self.supply.is_some()
    }

    fn mark_core_execution_complete(
        &mut self,
        rail_execution_sha256: String,
        supply_execution_sha256s: Vec<String>,
    ) -> Result<()> {
        if !self.executable_frame_present() {
            return Err("XMAP16 cannot complete without retained rail/supply handoffs".into());
        }
        if rail_execution_sha256.len() != 64
            || !rail_execution_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !matches!(supply_execution_sha256s.len(), 0 | 2)
            || supply_execution_sha256s.iter().any(|value| {
                value.len() != 64
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
        {
            return Err("XMAP16 execution receipt closure is invalid".into());
        }
        self.core_execution_complete = true;
        self.rail_execution_sha256 = Some(rail_execution_sha256);
        self.supply_execution_sha256s = supply_execution_sha256s;
        Ok(())
    }

    fn execution_complete(&self) -> bool {
        self.core_execution_complete && self.rail_execution_sha256.is_some()
    }

    fn execution_receipt_sha256s(&self) -> Result<(&str, &[String])> {
        if !self.execution_complete() {
            return Err("XMAP16 execution receipt closure is incomplete".into());
        }
        Ok((
            self.rail_execution_sha256
                .as_deref()
                .ok_or("XMAP16 railroad execution receipt disappeared")?,
            &self.supply_execution_sha256s,
        ))
    }

    fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }

    unsafe fn execute_rail<S>(
        &self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        scene_cb0: &S,
        target: &dyn map_viewer::NativeXmap16MainSceneTargetView,
        target_sha256: &str,
        external_resources: &[map_viewer::NativeXmap16RailExternalResourceView],
    ) -> Result<map_viewer::Xmap16RailExecutionReceipt>
    where
        S: map_viewer::NativeXmap16RailSceneConstantBufferView,
    {
        let frame = prepared.require_xmap16_rail_supply()?;
        frame.validate()?;
        viewer.validate()?;
        let rail = self
            .rail
            .as_ref()
            .ok_or("XMAP16 railroad handoff has not been retained")?;
        let target = map_viewer::NativeXmap16RailTargetView {
            target,
            target_sha256,
        };
        let shared = map_viewer::Xmap16RailSharedExecutionIdentity {
            content_manifest_sha256: prepared.envelope.content_manifest_sha256.clone(),
            foundation_content_operation_sha256: prepared
                .envelope
                .foundation_content_operation_sha256
                .clone(),
            viewer_state_sha256: snapshot::sha256::Sha256::hex(&serde_json::to_vec(viewer)?),
            main_scene_target_sha256: target_sha256.into(),
            map_copy_receipt_set_sha256: prepared.terrain.map_copy_receipt_set_sha256()?,
        };
        shared.validate()?;
        rail.draw_target_bound(
            frame,
            map_viewer::NativeXmap16RailDrawInputs {
                context: gfx.context.as_raw(),
                target,
                assignment: &frame.rail_stage,
                shared: &shared,
                foundation: &prepared.envelope,
                viewer,
                scene_cb0,
                external_resources,
            },
        )
    }

    unsafe fn execute_supply<S>(
        &mut self,
        gfx: &Gfx,
        prepared: &map_viewer::PreparedFoundationFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        scene_cb0: &S,
        target: &dyn map_viewer::NativeXmap16MainSceneTargetView,
        external_resources: &[map_viewer::NativeXmap16ExternalResourceView],
        point_lights_cb2: &map_viewer::NativeXmap16ExternalConstantBufferView,
    ) -> Result<(
        map_viewer::Xmap16SupplyArrowDrawReceipt,
        map_viewer::Xmap16SupplyArrowDrawReceipt,
    )>
    where
        S: map_viewer::NativeXmap16SceneConstantBufferView,
    {
        let frame = prepared.require_xmap16_rail_supply()?;
        frame.validate()?;
        viewer.validate()?;
        let schedule = map_viewer::Xmap16SupplyArrowScheduleAssignment::from_prepared_frame(frame)?;
        let application_elapsed_seconds =
            map_viewer::NativeXmap16SceneConstantBufferView::application_elapsed_seconds(scene_cb0);
        if !application_elapsed_seconds.is_finite() {
            return Err("XMAP16 SceneCB0 supplied non-finite application elapsed seconds".into());
        }
        let supply_cb3_inputs = frame
            .supply_mesh
            .ordered_draws
            .iter()
            .map(|draw| map_viewer::Xmap16SupplyArrowCb3SemanticInput {
                row_ordinal: draw.row_ordinal,
                application_elapsed_seconds,
            })
            .collect::<Vec<_>>();
        let supply = self
            .supply
            .as_mut()
            .ok_or("XMAP16 supply-arrow handoff has not been retained")?;
        let mut draw_supply = |pass| -> Result<map_viewer::Xmap16SupplyArrowDrawReceipt> {
            unsafe {
                supply.draw(map_viewer::NativeXmap16SupplyArrowDrawInputs {
                    context: gfx.context.as_raw(),
                    target,
                    prepared_frame: frame,
                    schedule: &schedule,
                    pass,
                    scene_cb0,
                    point_lights_cb2,
                    external_resources,
                    cb3_inputs: &supply_cb3_inputs,
                })
            }
        };
        let supply_normal = draw_supply(map_viewer::Xmap16SupplyArrowPass::Normal)?;
        let supply_see_through = draw_supply(map_viewer::Xmap16SupplyArrowPass::SeeThrough)?;
        Ok((supply_normal, supply_see_through))
    }

    fn retain_hub_frame(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        prepared: &map_viewer::PreparedFoundationFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        selection_provider: &dyn map_viewer::Xmap16HubViewerSelectionProvider,
        gui_scale: f32,
    ) -> Result<&map_viewer::Xmap16HubUploadReceipt> {
        let rail = prepared.require_xmap16_rail_supply()?;
        let packet =
            map_viewer::prepare_xmap16_hub_gui_packet(rail, viewer, selection_provider, gui_scale)?;
        if self.hub_renderer.is_none() {
            self.hub_renderer = Some(QuadRenderer::new(gfx)?);
        }
        if self.hub.is_none() {
            self.hub = Some(unsafe {
                map_viewer::NativeXmap16HubHandoff::create_on_device(
                    gfx.device.as_raw(),
                    resolver,
                    &packet,
                )?
            });
        } else {
            unsafe {
                self.hub
                    .as_mut()
                    .ok_or("XMAP16 hub handoff disappeared")?
                    .update_packet(gfx.device.as_raw(), resolver, &packet)?;
            }
        }
        self.hub_packet = Some(packet);
        Ok(self
            .hub
            .as_ref()
            .ok_or("XMAP16 hub handoff was not retained")?
            .receipt())
    }

    fn clear_hub_frame(&mut self) {
        self.hub_packet = None;
    }

    fn execute_hubs(&mut self, gfx: &Gfx) -> Result<Option<map_viewer::Xmap16HubExecutionReceipt>> {
        let Some(packet) = self.hub_packet.as_ref() else {
            return Ok(None);
        };
        let handoff = self
            .hub
            .as_ref()
            .ok_or("XMAP16 hub texture handoff has not been retained")?;
        let renderer = self
            .hub_renderer
            .as_ref()
            .ok_or("XMAP16 hub shared GUI renderer has not been retained")?;
        let identity_sha256 = {
            use sha2::Digest;
            format!(
                "{:x}",
                sha2::Sha256::digest(b"replay_app.product-hud-textured-quad.xmap16.a729.v1")
            )
        };
        let mut executor = ProductXmap16HubGuiExecutor {
            gfx,
            renderer,
            identity_sha256,
        };
        unsafe {
            handoff
                .execute_with(gfx.context.as_raw(), packet, &mut executor)
                .map(Some)
        }
    }
}

impl ProductTerrainHandoffRuntime {
    fn bind_political_dependencies(
        &mut self,
        gfx: &Gfx,
        political: &ProductPoliticalHandoffRuntime,
    ) -> Result<&map_viewer::Xmap04D3dHandoffReceipt> {
        let terrain = self
            .handoff
            .as_mut()
            .ok_or("XMAP04 terrain handoff is absent at XMAP05 dependency bind")?;
        let political = political
            .handoff
            .as_ref()
            .ok_or("XMAP05 political handoff is absent at terrain dependency bind")?;
        unsafe { terrain.bind_xmap05_dependencies(gfx.context.as_raw(), political) }
    }

    fn bind_water_dependencies(
        &mut self,
        gfx: &Gfx,
        water: &ProductWaterHandoffRuntime,
    ) -> Result<&map_viewer::Xmap04D3dHandoffReceipt> {
        let terrain = self
            .handoff
            .as_mut()
            .ok_or("XMAP04 terrain handoff is absent at XMAP07 dependency bind")?;
        let water = water
            .handoff
            .as_ref()
            .ok_or("XMAP07 water handoff is absent at terrain dependency bind")?;
        unsafe { terrain.bind_xmap07_dependencies(gfx.context.as_raw(), water) }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProductMapShippingRoute {
    Window,
    Headless,
    Benchmark,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProductXmap07ShippingIngress {
    route: ProductMapShippingRoute,
    displayed_hour: i64,
    payload_blob_sha256: String,
    authority_sha256: String,
}

fn product_xmap07_shipping_ingress(
    route: ProductMapShippingRoute,
    world: &WorldFrame,
) -> Result<ProductXmap07ShippingIngress> {
    let carrier = world.xmap07_snow_mud_runtime().ok_or_else(|| {
        format!(
            "XMAP07 {:?} shipping route requires the passive global SnowMud carrier",
            route
        )
    })?;
    carrier
        .validate_for_hour(world.hour_index())
        .map_err(|error| {
            format!(
                "XMAP07 {:?} shipping route rejected passive global SnowMud: {error}",
                route
            )
        })?;
    Ok(ProductXmap07ShippingIngress {
        route,
        displayed_hour: carrier.displayed_hour,
        payload_blob_sha256: carrier.payload_blob_sha256.clone(),
        authority_sha256: carrier.authority_sha256.clone(),
    })
}

pub(crate) struct ProductMapCompositionSession {
    shipping_route: ProductMapShippingRoute,
    owned_gfx: Option<Gfx>,
    viewport_width: u32,
    viewport_height: u32,
    target_device_identity: Option<usize>,
    gpu_completion_query: Option<crate::d3d::GpuCompletionQuery>,
    foundation: crate::validate::ProductMapFoundationSession,
    terrain: ProductTerrainHandoffRuntime,
    political: ProductPoliticalHandoffRuntime,
    lighting: ProductLightingHandoffRuntime,
    trees: ProductTreeHandoffRuntime,
    water: ProductWaterHandoffRuntime,
    thin_borders: ProductThinBorderHandoffRuntime,
    rivers_straits: ProductRiversStraitsHandoffRuntime,
    xmap10: ProductXmap10HandoffRuntime,
    xmap12: ProductXmap12HandoffRuntime,
    xmap13_units: ProductXmap13UnitRuntime,
    xmap13_counters: crate::counter_gpu::Xmap13CounterGpuPass,
    orders: ProductOrdersHandoffRuntime,
    rail: ProductRailHandoffRuntime,
    combat: crate::combat_gpu::CombatGpuPass,
    furniture: Option<FurnitureRenderPass>,
    presentation: map_viewer::Xmap10ViewerPresentationState,
    pending_presentation_checkpoint: Option<ProductMapPresentationCheckpoint>,
    topology_sha256: String,
    schedule_sha256: String,
    schedule: map_viewer::AssignedStageSchedule,
    map_width: u32,
    map_height: u32,
    texture_quality: u8,
    last_prepared: Option<map_viewer::PreparedFoundationFrame>,
}

#[derive(Clone, Debug)]
struct ProductMapPresentationCheckpoint {
    presentation: map_viewer::Xmap10ViewerPresentationState,
    map_label_alpha: Option<map_viewer::MapNameAlphaState>,
}

fn reject_product_map_presentation(
    presentation: &mut map_viewer::Xmap10ViewerPresentationState,
    xmap10: &mut ProductXmap10HandoffRuntime,
    checkpoint: ProductMapPresentationCheckpoint,
) {
    *presentation = checkpoint.presentation;
    xmap10.map_label_alpha = checkpoint.map_label_alpha;
    xmap10.discard_unpresented_temporal_frame();
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum Xmap16SupplyCompositionDisposition {
    Present {
        normal_receipt_sha256: String,
        see_through_receipt_sha256: String,
    },
    MapModeInactive {
        prepared_map_modes_sha256: String,
    },
}

impl Xmap16SupplyCompositionDisposition {
    fn validate(&self, prepared_map_modes_sha256: &str, supply_view_active: bool) -> Result<()> {
        match self {
            Self::Present {
                normal_receipt_sha256,
                see_through_receipt_sha256,
            } => {
                if !supply_view_active {
                    return Err("XMAP16 supply execution is present while its authenticated map mode is inactive".into());
                }
                require_xmap22_sha256(normal_receipt_sha256, "XMAP16 normal supply execution")?;
                require_xmap22_sha256(
                    see_through_receipt_sha256,
                    "XMAP16 see-through supply execution",
                )?;
            }
            Self::MapModeInactive {
                prepared_map_modes_sha256: witness,
            } => {
                if supply_view_active || witness != prepared_map_modes_sha256 {
                    return Err("XMAP16 inactive supply disposition is not bound to the authenticated map-mode state".into());
                }
                require_xmap22_sha256(witness, "XMAP16 inactive-map-mode witness")?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum Xmap16HubCompositionDisposition {
    Present { execution_receipt_sha256: String },
    NoObserverCountry { viewer_state_sha256: String },
}

impl Xmap16HubCompositionDisposition {
    fn validate(&self, viewer_state_sha256: &str) -> Result<()> {
        match self {
            Self::Present {
                execution_receipt_sha256,
            } => require_xmap22_sha256(execution_receipt_sha256, "XMAP16 hub execution")?,
            Self::NoObserverCountry {
                viewer_state_sha256: witness,
            } => {
                if witness != viewer_state_sha256 {
                    return Err("XMAP16 no-observer disposition is not bound to the authenticated viewer state".into());
                }
                require_xmap22_sha256(witness, "XMAP16 no-observer witness")?;
            }
        }
        Ok(())
    }
}

fn require_xmap22_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("XMAP22 {label} has an invalid SHA-256").into());
    }
    Ok(())
}

fn validate_xmap22_foundation_disposition(
    family: map_viewer::MapVisualFamily,
    disposition: &map_viewer::FamilyDisposition,
) -> Result<()> {
    match disposition {
        map_viewer::FamilyDisposition::Present {
            payload_schema,
            producer_registration_sha256,
            payload_sha256,
            ordered_operations,
            ordered_used_content,
        } => {
            if payload_schema.trim().is_empty() {
                return Err(format!("XMAP22 {family:?} has a blank payload schema").into());
            }
            require_xmap22_sha256(producer_registration_sha256, "family producer registration")?;
            require_xmap22_sha256(payload_sha256, "family payload")?;
            let mut operation_ids = BTreeSet::new();
            let mut prior_order = None;
            for operation in ordered_operations {
                if operation.operation_id.trim().is_empty()
                    || !operation_ids.insert(operation.operation_id.as_str())
                    || prior_order.is_some_and(|prior| operation.global_order <= prior)
                {
                    return Err(format!(
                        "XMAP22 {family:?} operations are blank, duplicated, or not strictly ordered"
                    )
                    .into());
                }
                require_xmap22_sha256(&operation.program_sha256, "operation program")?;
                require_xmap22_sha256(
                    &operation.ordered_resources_sha256,
                    "operation resource order",
                )?;
                require_xmap22_sha256(&operation.target_sha256, "operation target")?;
                prior_order = Some(operation.global_order);
            }
            let mut used_paths = BTreeSet::new();
            for content in ordered_used_content {
                content.validate().map_err(|error| {
                    format!("XMAP22 {family:?} has invalid UsedContent: {error}")
                })?;
                if !used_paths.insert(content.normalized_game_relative_path.as_str()) {
                    return Err(format!("XMAP22 {family:?} repeats a UsedContent path").into());
                }
            }
        }
        map_viewer::FamilyDisposition::AuthenticatedEmpty {
            producer_registration_sha256,
            enumerator_witness_sha256,
        } => {
            require_xmap22_sha256(
                producer_registration_sha256,
                "empty-family producer registration",
            )?;
            require_xmap22_sha256(enumerator_witness_sha256, "empty-family enumerator")?;
        }
        map_viewer::FamilyDisposition::NotApplicable {
            applicability_witness_sha256,
            reason,
        } => {
            require_xmap22_sha256(applicability_witness_sha256, "not-applicable witness")?;
            if reason.trim().is_empty() {
                return Err(format!("XMAP22 {family:?} has a blank not-applicable reason").into());
            }
        }
        map_viewer::FamilyDisposition::Unavailable { gap } => {
            require_xmap22_sha256(&gap.witness_sha256, "unavailable-family witness")?;
            if gap.gap_id.trim().is_empty() || gap.reason.trim().is_empty() {
                return Err(
                    format!("XMAP22 {family:?} is unavailable without a named gap").into(),
                );
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "execution", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Xmap22FamilyExecutionDisposition {
    Executed {
        execution_sha256: String,
    },
    MeasuredEmpty {
        execution_sha256: String,
        enumerator_witness_sha256: String,
    },
    NotApplicable {
        applicability_witness_sha256: String,
    },
    Unavailable {
        gap_id: String,
        reason: String,
        witness_sha256: String,
    },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Xmap22FamilyExecutionRow {
    pub(crate) family: map_viewer::MapVisualFamily,
    pub(crate) owner_packet: String,
    pub(crate) foundation_disposition: map_viewer::FamilyDisposition,
    pub(crate) execution: Xmap22FamilyExecutionDisposition,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Xmap22PresentationStatus {
    OffscreenReadbackMeasured,
    WindowPendingPresent,
    WindowPresented,
    WindowSkipped,
    WindowOccluded,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductMapCompositionReceipt {
    pub(crate) schema: String,
    pub(crate) hour_index: i64,
    pub(crate) viewport_width: u32,
    pub(crate) viewport_height: u32,
    pub(crate) content_manifest_sha256: String,
    pub(crate) product_modset_sha256: String,
    pub(crate) effective_vfs_manifest_sha256: String,
    pub(crate) replay_frame_identity_sha256: String,
    pub(crate) viewer_state_sha256: String,
    pub(crate) xmap15_viewer_selection_sha256: String,
    pub(crate) assigned_stage_topology_sha256: String,
    pub(crate) assigned_stage_schedule_sha256: String,
    pub(crate) foundation_envelope_sha256: String,
    pub(crate) foundation_content_operation_sha256: String,
    pub(crate) xmap04_terrain_execution_sha256: String,
    pub(crate) xmap05_resource_execution_sha256: String,
    pub(crate) xmap06_thin_borders_execution_sha256: String,
    pub(crate) xmap07_water_execution_sha256: String,
    pub(crate) xmap07_snow_mud_authority_sha256: String,
    pub(crate) xmap08_rivers_straits_execution_sha256: String,
    pub(crate) xmap09_lighting_execution_sha256: String,
    pub(crate) xmap10_postfx_receipt_sha256: String,
    pub(crate) xmap20_scene_receipt_sha256: String,
    pub(crate) xmap11_tree_execution_sha256: String,
    pub(crate) xmap12_execution_sha256: String,
    pub(crate) xmap13_unit_execution_sha256: String,
    pub(crate) xmap13_counter_execution_sha256: String,
    pub(crate) xmap14_execution_sha256: String,
    pub(crate) xmap15_execution_sha256: String,
    pub(crate) xmap16_rail_execution_sha256: String,
    pub(crate) prepared_map_modes_sha256: String,
    pub(crate) prepared_map_mode_composition: String,
    pub(crate) xmap16_supply: Xmap16SupplyCompositionDisposition,
    pub(crate) xmap17_execution_sha256: String,
    pub(crate) xmap19_weather_execution_sha256: String,
    pub(crate) xmap21_execution_sha256: String,
    pub(crate) xmap16_hub: Xmap16HubCompositionDisposition,
    pub(crate) mandatory_family_count: u32,
    pub(crate) mandatory_family_execution_census: Vec<Xmap22FamilyExecutionRow>,
    pub(crate) mandatory_family_execution_census_sha256: String,
    pub(crate) foundation_complete: bool,
    pub(crate) execution_complete: bool,
    pub(crate) final_pixels_measured: bool,
    pub(crate) presented_to_product_target: bool,
    pub(crate) presentation_status: Xmap22PresentationStatus,
    pub(crate) final_target_sha256: String,
    pub(crate) final_pixels_sha256: String,
    pub(crate) deterministic_sha256: String,
}

impl ProductMapCompositionReceipt {
    fn seal(mut self) -> Result<Self> {
        self.deterministic_sha256.clear();
        self.deterministic_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(&self)?);
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.schema != "hoi4.replay-map-composition.xmap22.v4"
            || self.viewport_width == 0
            || self.viewport_height == 0
            || self.mandatory_family_count != u32::try_from(map_viewer::MapVisualFamily::ALL.len())?
            || self.mandatory_family_execution_census.len()
                != map_viewer::MapVisualFamily::ALL.len()
            || !self.foundation_complete
            || !self.execution_complete
            || !self.final_pixels_measured
        {
            return Err("XMAP22 composition receipt is incomplete".into());
        }
        let expected_census_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(
            &self.mandatory_family_execution_census,
        )?);
        if self.mandatory_family_execution_census_sha256 != expected_census_sha256 {
            return Err("XMAP22 mandatory-family census digest mismatch".into());
        }
        for (expected, row) in map_viewer::MapVisualFamily::ALL
            .iter()
            .zip(&self.mandatory_family_execution_census)
        {
            if &row.family != expected || row.owner_packet != expected.owner_packet() {
                return Err("XMAP22 mandatory-family census identity/order mismatch".into());
            }
            validate_xmap22_foundation_disposition(row.family, &row.foundation_disposition)?;
            match (&row.foundation_disposition, &row.execution) {
                (
                    map_viewer::FamilyDisposition::Present { .. },
                    Xmap22FamilyExecutionDisposition::Executed { execution_sha256 },
                ) => require_xmap22_sha256(execution_sha256, "family execution")?,
                (
                    map_viewer::FamilyDisposition::AuthenticatedEmpty {
                        enumerator_witness_sha256,
                        ..
                    },
                    Xmap22FamilyExecutionDisposition::MeasuredEmpty {
                        execution_sha256,
                        enumerator_witness_sha256: execution_witness,
                    },
                ) if enumerator_witness_sha256 == execution_witness => {
                    require_xmap22_sha256(execution_sha256, "measured-empty execution")?;
                    require_xmap22_sha256(execution_witness, "measured-empty enumerator")?;
                }
                (
                    map_viewer::FamilyDisposition::NotApplicable {
                        applicability_witness_sha256,
                        ..
                    },
                    Xmap22FamilyExecutionDisposition::NotApplicable {
                        applicability_witness_sha256: execution_witness,
                    },
                ) if applicability_witness_sha256 == execution_witness => {
                    require_xmap22_sha256(execution_witness, "not-applicable witness")?;
                }
                // EXCLUDED AND NAMED, and the two halves must name the SAME
                // gap. This used to be `unreachable!`, on the belief that an
                // unavailable foundation row never reached execution matching;
                // it does -- `xmap22_family_execution_rows` builds an
                // `Unavailable` execution for exactly this case, and XMAP14 now
                // produces one whenever the install's authored content cannot
                // build the combat layer. A validator that panics on a state
                // its own producer emits is not a validator (Codex review of
                // batch 3).
                //
                // Equality across all three fields is the whole check: a
                // receipt that said "unavailable" while naming a different gap,
                // reason or witness than the foundation row would let a reader
                // attribute the missing layer to the wrong cause.
                (
                    map_viewer::FamilyDisposition::Unavailable { gap },
                    Xmap22FamilyExecutionDisposition::Unavailable {
                        gap_id,
                        reason,
                        witness_sha256,
                    },
                ) if &gap.gap_id == gap_id
                    && &gap.reason == reason
                    && &gap.witness_sha256 == witness_sha256 =>
                {
                    require_xmap22_sha256(witness_sha256, "unavailable-family witness")?;
                }
                _ => {
                    return Err(
                        "XMAP22 family execution does not match its foundation disposition".into(),
                    )
                }
            }
        }
        match self.presentation_status {
            Xmap22PresentationStatus::OffscreenReadbackMeasured
            | Xmap22PresentationStatus::WindowPresented
                if self.presented_to_product_target => {}
            Xmap22PresentationStatus::WindowPendingPresent
            | Xmap22PresentationStatus::WindowSkipped
            | Xmap22PresentationStatus::WindowOccluded
                if !self.presented_to_product_target => {}
            _ => return Err("XMAP22 presentation status/boolean mismatch".into()),
        }
        let known_map_mode = matches!(
            self.prepared_map_mode_composition.as_str(),
            "political"
                | "strategic_air"
                | "strategic_navy"
                | "intelligence"
                | "state_owner"
                | "supply"
                | "resistance"
                | "compliance"
                | "resource_furniture"
                | "diplomacy"
                | "factions"
                | "players"
                | "infrastructure"
                | "manpower"
                | "ideology"
                | "terrain"
                | "raid_furniture"
                | "scripted_two_layer"
        );
        if !known_map_mode {
            return Err(
                "XMAP22 composition receipt carries an unknown authenticated map-mode composition"
                    .into(),
            );
        }
        self.xmap16_supply.validate(
            &self.prepared_map_modes_sha256,
            self.prepared_map_mode_composition == "supply",
        )?;
        self.xmap16_hub.validate(&self.viewer_state_sha256)?;
        for value in [
            &self.content_manifest_sha256,
            &self.product_modset_sha256,
            &self.effective_vfs_manifest_sha256,
            &self.replay_frame_identity_sha256,
            &self.viewer_state_sha256,
            &self.xmap15_viewer_selection_sha256,
            &self.assigned_stage_topology_sha256,
            &self.assigned_stage_schedule_sha256,
            &self.foundation_envelope_sha256,
            &self.foundation_content_operation_sha256,
            &self.xmap04_terrain_execution_sha256,
            &self.xmap05_resource_execution_sha256,
            &self.xmap06_thin_borders_execution_sha256,
            &self.xmap07_water_execution_sha256,
            &self.xmap07_snow_mud_authority_sha256,
            &self.xmap08_rivers_straits_execution_sha256,
            &self.xmap09_lighting_execution_sha256,
            &self.xmap10_postfx_receipt_sha256,
            &self.xmap20_scene_receipt_sha256,
            &self.xmap11_tree_execution_sha256,
            &self.xmap12_execution_sha256,
            &self.xmap13_unit_execution_sha256,
            &self.xmap13_counter_execution_sha256,
            &self.xmap14_execution_sha256,
            &self.xmap15_execution_sha256,
            &self.xmap16_rail_execution_sha256,
            &self.prepared_map_modes_sha256,
            &self.xmap17_execution_sha256,
            &self.xmap19_weather_execution_sha256,
            &self.xmap21_execution_sha256,
            &self.mandatory_family_execution_census_sha256,
            &self.final_target_sha256,
            &self.final_pixels_sha256,
            &self.deterministic_sha256,
        ] {
            require_xmap22_sha256(value, "composition receipt field")?;
        }
        let reconstructed_foundation = xmap22_reconstructed_foundation_envelope(self)?;
        let reconstructed_foundation_sha256 = reconstructed_foundation
            .deterministic_sha256()
            .map_err(|error| format!("XMAP22 reconstructed foundation digest: {error}"))?;
        if self.foundation_envelope_sha256 != reconstructed_foundation_sha256 {
            return Err(
                "XMAP22 foundation envelope digest contradicts its embedded family rows".into(),
            );
        }
        let expected_execution_witnesses = xmap22_expected_family_execution_witnesses(self)?;
        if expected_execution_witnesses.len() != map_viewer::MapVisualFamily::ALL.len() {
            return Err("XMAP22 independently derived family witnesses are incomplete".into());
        }
        for row in &self.mandatory_family_execution_census {
            let expected = expected_execution_witnesses
                .get(&row.family)
                .ok_or("XMAP22 family census has no independently derived witness")?;
            match &row.execution {
                Xmap22FamilyExecutionDisposition::Executed { execution_sha256 }
                | Xmap22FamilyExecutionDisposition::MeasuredEmpty {
                    execution_sha256, ..
                } if execution_sha256 == expected => {}
                Xmap22FamilyExecutionDisposition::NotApplicable { .. } => {}
                // A family that did not execute has no PACKET receipt to
                // contradict -- it has a NAMED GAP, already checked against the
                // foundation row field for field above. Left out of this match
                // entirely, an excluded-and-named layer failed the receipt it is
                // supposed to appear in.
                //
                // But it must not be waved through either: the producer puts the
                // gap's witness into the family's top-level execution field, and
                // if nothing REQUIRES that, the field is unbound -- a receipt
                // could carry an arbitrary canonical digest there, reseal, and
                // still be accepted while advertising a complete frame (Codex
                // review of batch 3, round 2). So the witness is bound here.
                Xmap22FamilyExecutionDisposition::Unavailable {
                    witness_sha256, ..
                } => {
                    // AN UNBOUND FAMILY IS A REFUSAL, not a pass. Returning
                    // `None` for the families whose unavailable path is not
                    // built yet, and then accepting them, reproduced the very
                    // hole this arm exists to close for all nineteen of them:
                    // a resealed receipt could mark any of them unavailable
                    // while carrying an arbitrary flat execution digest, and
                    // still advertise a complete frame (Codex review of batch
                    // 3, round 3). A family joins this list when its
                    // unavailable path is actually built.
                    let Some(bound) =
                        xmap22_family_unavailable_witness_field(self, row.family)
                    else {
                        return Err(format!(
                            "XMAP22 {:?} is unavailable, which this receipt has no bound \
                             execution field for",
                            row.family
                        )
                        .into());
                    };
                    if bound != witness_sha256 {
                        return Err(format!(
                            "XMAP22 {:?} is unavailable but its execution field is not the gap witness",
                            row.family
                        )
                        .into());
                    }
                }
                _ => {
                    return Err(format!(
                        "XMAP22 {:?} execution witness contradicts its packet receipts",
                        row.family
                    )
                    .into())
                }
            }
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256
            != snapshot::sha256::Sha256::hex(&serde_json::to_vec(&unsigned)?)
        {
            return Err("XMAP22 composition receipt digest mismatch".into());
        }
        Ok(())
    }

    pub(crate) fn finish_window_present(
        mut self,
        outcome: crate::d3d::PresentOutcome,
    ) -> Result<Self> {
        if self.presentation_status != Xmap22PresentationStatus::WindowPendingPresent
            || self.presented_to_product_target
        {
            return Err("XMAP22 window receipt is not pending presentation".into());
        }
        self.presentation_status = match outcome {
            crate::d3d::PresentOutcome::Presented => Xmap22PresentationStatus::WindowPresented,
            crate::d3d::PresentOutcome::Skipped => Xmap22PresentationStatus::WindowSkipped,
            crate::d3d::PresentOutcome::Occluded => Xmap22PresentationStatus::WindowOccluded,
        };
        self.presented_to_product_target = outcome == crate::d3d::PresentOutcome::Presented;
        self.seal()
    }
}

fn xmap22_reconstructed_foundation_envelope(
    receipt: &ProductMapCompositionReceipt,
) -> Result<map_viewer::FoundationRenderEnvelope> {
    let envelope = map_viewer::FoundationRenderEnvelope {
        schema: map_viewer::FOUNDATION_RENDER_SCHEMA.into(),
        hour_index: receipt.hour_index,
        content_manifest_sha256: receipt.content_manifest_sha256.clone(),
        product_modset_sha256: receipt.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: receipt.effective_vfs_manifest_sha256.clone(),
        foundation_content_operation_sha256: receipt.foundation_content_operation_sha256.clone(),
        families: receipt
            .mandatory_family_execution_census
            .iter()
            .map(|row| map_viewer::FamilyCoverageRow {
                family: row.family,
                owner_packet: row.owner_packet.clone(),
                disposition: row.foundation_disposition.clone(),
            })
            .collect(),
    };
    envelope
        .validate()
        .map_err(|error| format!("XMAP22 reconstructed foundation: {error}"))?;
    Ok(envelope)
}

/// What a family's execution witness IS when the layer was excluded and named:
/// the receipt digest if the layer executed, and otherwise the named gap's own
/// witness.
///
/// A pure function so the compositor's choice can be tested without a GPU. It
/// is the whole of the rule at the two sites in `compose_product_map_frame`
/// that had to stop demanding a receipt, and the case that matters is the last
/// one: no receipt AND no gap is a refusal, never a fabricated witness.
fn xmap22_execution_witness_or_named_gap(
    receipt_sha256: Option<&str>,
    gap_witness_sha256: Option<&str>,
    family: &str,
) -> Result<String> {
    match (receipt_sha256, gap_witness_sha256) {
        (Some(receipt), _) => Ok(receipt.to_string()),
        (None, Some(gap_witness)) => Ok(gap_witness.to_string()),
        (None, None) => Err(format!(
            "XMAP22 complete frame has no {family} execution receipt and no named {family} gap"
        )
        .into()),
    }
}

/// The receipt's own top-level execution field for a family, where it has one.
///
/// An unavailable family's execution witness is the NAMED GAP's witness, and it
/// travels twice: once in the census row and once in the receipt's flat field
/// that consumers read. Nothing required the two to agree, so the flat field was
/// unbound for exactly the families that did not execute (Codex review of batch
/// 3, round 2).
///
/// `None` means this family has NO BOUND FIELD YET, which the caller treats as
/// a refusal rather than a pass: every family has receipt-derived execution
/// inputs, so an unbound one is a family whose unavailable path has not been
/// built, not a family that is exempt. Add a row here when that path is built.
fn xmap22_family_unavailable_witness_field(
    receipt: &ProductMapCompositionReceipt,
    family: map_viewer::MapVisualFamily,
) -> Option<&str> {
    match family {
        map_viewer::MapVisualFamily::CombatMarkers => {
            Some(receipt.xmap14_execution_sha256.as_str())
        }
        _ => None,
    }
}

fn xmap22_expected_family_execution_witnesses(
    receipt: &ProductMapCompositionReceipt,
) -> Result<BTreeMap<map_viewer::MapVisualFamily, String>> {
    let supply_disposition_sha256 =
        snapshot::sha256::Sha256::hex(&serde_json::to_vec(&receipt.xmap16_supply)?);
    let hub_disposition_sha256 =
        snapshot::sha256::Sha256::hex(&serde_json::to_vec(&receipt.xmap16_hub)?);
    let mut witnesses = BTreeMap::new();
    let mut insert = |family, ordered: &[&str]| -> Result<()> {
        let previous = witnesses.insert(family, xmap22_family_execution_sha256(family, ordered)?);
        if previous.is_some() {
            return Err("XMAP22 independently derived a duplicate family witness".into());
        }
        Ok(())
    };
    insert(
        map_viewer::MapVisualFamily::CameraView,
        &[
            &receipt.viewer_state_sha256,
            &receipt.xmap04_terrain_execution_sha256,
        ],
    )?;
    insert(
        map_viewer::MapVisualFamily::Terrain,
        &[&receipt.xmap04_terrain_execution_sha256],
    )?;
    for family in [
        map_viewer::MapVisualFamily::PoliticalFillGradients,
        map_viewer::MapVisualFamily::FogOfWar,
    ] {
        insert(
            family,
            &[
                &receipt.xmap05_resource_execution_sha256,
                &receipt.xmap04_terrain_execution_sha256,
            ],
        )?;
    }
    insert(
        map_viewer::MapVisualFamily::ThinBorders,
        &[&receipt.xmap06_thin_borders_execution_sha256],
    )?;
    insert(
        map_viewer::MapVisualFamily::WaterIce,
        &[
            &receipt.xmap07_water_execution_sha256,
            &receipt.xmap07_snow_mud_authority_sha256,
        ],
    )?;
    insert(
        map_viewer::MapVisualFamily::RiversStraits,
        &[&receipt.xmap08_rivers_straits_execution_sha256],
    )?;
    insert(
        map_viewer::MapVisualFamily::LightingShadows,
        &[&receipt.xmap09_lighting_execution_sha256],
    )?;
    insert(
        map_viewer::MapVisualFamily::GradingPostFx,
        &[&receipt.xmap10_postfx_receipt_sha256],
    )?;
    insert(
        map_viewer::MapVisualFamily::Trees,
        &[&receipt.xmap11_tree_execution_sha256],
    )?;
    insert(
        map_viewer::MapVisualFamily::CitiesBuildingsMapObjects,
        &[&receipt.xmap12_execution_sha256],
    )?;
    insert(
        map_viewer::MapVisualFamily::UnitsCounters,
        &[
            &receipt.xmap13_unit_execution_sha256,
            &receipt.xmap13_counter_execution_sha256,
        ],
    )?;
    insert(
        map_viewer::MapVisualFamily::CombatMarkers,
        &[&receipt.xmap14_execution_sha256],
    )?;
    insert(
        map_viewer::MapVisualFamily::OrdersFrontsArrows,
        &[&receipt.xmap15_execution_sha256],
    )?;
    insert(
        map_viewer::MapVisualFamily::RailSupply,
        &[
            &receipt.xmap16_rail_execution_sha256,
            &supply_disposition_sha256,
            &hub_disposition_sha256,
        ],
    )?;
    insert(
        map_viewer::MapVisualFamily::LabelsMapText,
        &[&receipt.xmap17_execution_sha256],
    )?;
    insert(
        map_viewer::MapVisualFamily::MapModes,
        &[
            &receipt.prepared_map_modes_sha256,
            &receipt.xmap04_terrain_execution_sha256,
        ],
    )?;
    insert(
        map_viewer::MapVisualFamily::WeatherAtmosphere,
        &[
            &receipt.xmap19_weather_execution_sha256,
            &receipt.xmap20_scene_receipt_sha256,
        ],
    )?;
    insert(
        map_viewer::MapVisualFamily::ParticlesEffects,
        &[&receipt.xmap20_scene_receipt_sha256],
    )?;
    insert(
        map_viewer::MapVisualFamily::MapFurnitureIcons,
        &[&receipt.xmap21_execution_sha256],
    )?;
    Ok(witnesses)
}

fn build_xmap22_family_execution_census(
    envelope: &map_viewer::FoundationRenderEnvelope,
    execution_witnesses: &BTreeMap<map_viewer::MapVisualFamily, String>,
) -> Result<Vec<Xmap22FamilyExecutionRow>> {
    envelope
        .validate()
        .map_err(|error| format!("XMAP22 foundation census: {error}"))?;
    if execution_witnesses.len() != map_viewer::MapVisualFamily::ALL.len()
        || map_viewer::MapVisualFamily::ALL
            .iter()
            .any(|family| !execution_witnesses.contains_key(family))
    {
        return Err(
            "XMAP22 execution witnesses do not cover the independent mandatory-family census"
                .into(),
        );
    }
    envelope
        .families
        .iter()
        .map(|foundation| {
            let execution_sha256 = execution_witnesses
                .get(&foundation.family)
                .ok_or("XMAP22 mandatory family has no execution witness")?
                .clone();
            require_xmap22_sha256(&execution_sha256, "mandatory-family execution witness")?;
            let execution = match &foundation.disposition {
                map_viewer::FamilyDisposition::Present { .. } => {
                    Xmap22FamilyExecutionDisposition::Executed { execution_sha256 }
                }
                map_viewer::FamilyDisposition::AuthenticatedEmpty {
                    enumerator_witness_sha256,
                    ..
                } => Xmap22FamilyExecutionDisposition::MeasuredEmpty {
                    execution_sha256,
                    enumerator_witness_sha256: enumerator_witness_sha256.clone(),
                },
                map_viewer::FamilyDisposition::NotApplicable {
                    applicability_witness_sha256,
                    ..
                } => Xmap22FamilyExecutionDisposition::NotApplicable {
                    applicability_witness_sha256: applicability_witness_sha256.clone(),
                },
                map_viewer::FamilyDisposition::Unavailable { gap } => {
                    require_xmap22_sha256(&gap.witness_sha256, "unavailable-family witness")?;
                    if gap.gap_id.trim().is_empty() || gap.reason.trim().is_empty() {
                        return Err(format!(
                            "XMAP22 mandatory family {:?} is unavailable without a named gap",
                            foundation.family
                        )
                        .into());
                    }
                    Xmap22FamilyExecutionDisposition::Unavailable {
                        gap_id: gap.gap_id.clone(),
                        reason: gap.reason.clone(),
                        witness_sha256: gap.witness_sha256.clone(),
                    }
                }
            };
            Ok(Xmap22FamilyExecutionRow {
                family: foundation.family,
                owner_packet: foundation.family.owner_packet().into(),
                foundation_disposition: foundation.disposition.clone(),
                execution,
            })
        })
        .collect()
}

fn xmap22_family_execution_sha256(
    family: map_viewer::MapVisualFamily,
    ordered_receipt_sha256s: &[&str],
) -> Result<String> {
    if ordered_receipt_sha256s.is_empty() {
        return Err("XMAP22 family execution witness has no packet-native receipt".into());
    }
    for receipt in ordered_receipt_sha256s {
        require_xmap22_sha256(receipt, "packet-native family receipt")?;
    }
    Ok(snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(
        "hoi4.replay-map-composition.xmap22.family-execution.v1",
        family,
        ordered_receipt_sha256s,
    ))?))
}

impl ProductMapCompositionSession {
    pub(crate) fn start_for_headless(
        resolver: &GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        manifest: &replay_core::ContentManifest,
        terrain_options: map_viewer::TerrainLodCullOptions,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Self> {
        Self::start_inner(
            resolver,
            content_admission,
            manifest,
            terrain_options,
            viewport_width,
            viewport_height,
            ProductMapShippingRoute::Headless,
            true,
        )
    }

    pub(crate) fn start_for_benchmark(
        resolver: &GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        manifest: &replay_core::ContentManifest,
        terrain_options: map_viewer::TerrainLodCullOptions,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Self> {
        Self::start_inner(
            resolver,
            content_admission,
            manifest,
            terrain_options,
            viewport_width,
            viewport_height,
            ProductMapShippingRoute::Benchmark,
            true,
        )
    }

    /// `start` only in target ownership.
    pub(crate) fn start_for_window_target(
        resolver: &GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        manifest: &replay_core::ContentManifest,
        terrain_options: map_viewer::TerrainLodCullOptions,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Self> {
        Self::start_inner(
            resolver,
            content_admission,
            manifest,
            terrain_options,
            viewport_width,
            viewport_height,
            ProductMapShippingRoute::Window,
            false,
        )
    }

    fn start_inner(
        resolver: &GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        manifest: &replay_core::ContentManifest,
        terrain_options: map_viewer::TerrainLodCullOptions,
        viewport_width: u32,
        viewport_height: u32,
        shipping_route: ProductMapShippingRoute,
        create_offscreen_target: bool,
    ) -> Result<Self> {
        let (topology, schedule) = crate::validate::product_foundation_assigned_stage_plan()?;
        let foundation =
            crate::validate::ProductMapFoundationSession::start_with_assigned_stage_schedule(
                resolver,
                content_admission,
                manifest,
                terrain_options,
                &topology,
                schedule.clone(),
            )?;
        let (map_width, map_height) =
            map_viewer::foundation_map_dimensions(resolver, content_admission)?;
        let owned_gfx = create_offscreen_target
            .then(|| Gfx::create_offscreen(viewport_width, viewport_height))
            .transpose()?;
        Ok(Self {
            shipping_route,
            furniture: None,
            xmap13_counters: crate::counter_gpu::Xmap13CounterGpuPass::new(resolver.clone()),
            combat: crate::combat_gpu::CombatGpuPass::new(resolver.clone()),
            presentation: map_viewer::Xmap10ViewerPresentationState::new()?,
            pending_presentation_checkpoint: None,
            topology_sha256: topology.topology_sha256,
            schedule_sha256: schedule.schedule_sha256.clone(),
            schedule,
            foundation,
            owned_gfx,
            viewport_width,
            viewport_height,
            target_device_identity: None,
            gpu_completion_query: None,
            terrain: ProductTerrainHandoffRuntime::default(),
            political: ProductPoliticalHandoffRuntime::default(),
            lighting: ProductLightingHandoffRuntime::default(),
            trees: ProductTreeHandoffRuntime::default(),
            water: ProductWaterHandoffRuntime::default(),
            thin_borders: ProductThinBorderHandoffRuntime::default(),
            rivers_straits: ProductRiversStraitsHandoffRuntime::default(),
            xmap10: ProductXmap10HandoffRuntime::default(),
            xmap12: ProductXmap12HandoffRuntime::default(),
            xmap13_units: ProductXmap13UnitRuntime::default(),
            orders: ProductOrdersHandoffRuntime::default(),
            rail: ProductRailHandoffRuntime::default(),
            map_width,
            map_height,
            texture_quality: terrain_options.texture_quality,
            last_prepared: None,
        })
    }

    pub(crate) fn adapter_identity(&self) -> Result<crate::d3d::GpuAdapterIdentity> {
        self.owned_gfx
            .as_ref()
            .ok_or("XMAP22 window-target session does not own its adapter")?
            .adapter_identity()
    }

    pub(crate) fn resize_window_target(&mut self, width: u32, height: u32) -> Result<()> {
        if self.owned_gfx.is_some() {
            return Err("XMAP22 offscreen session cannot adopt a window resize".into());
        }
        if width == 0 || height == 0 {
            return Ok(());
        }
        self.viewport_width = width;
        self.viewport_height = height;
        Ok(())
    }

    pub(crate) fn read_target(&self) -> Result<Vec<u8>> {
        self.owned_gfx
            .as_ref()
            .ok_or("XMAP22 window-target session does not own its readback target")?
            .read_target()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute(
        &mut self,
        resolver: &GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        manifest: &replay_core::ContentManifest,
        world: &WorldFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        xmap15_selection: &map_viewer::Xmap15ViewerSelection,
        localisation_language: &str,
        delta_seconds: f32,
        transport_discontinuity: bool,
        day_night_enabled: bool,
        gui_scale: f32,
    ) -> Result<ProductMapCompositionReceipt> {
        let gfx = self
            .owned_gfx
            .take()
            .ok_or("XMAP22 window-target session requires execute_on_target")?;
        let outcome = self.execute_on_target(
            &gfx,
            resolver,
            content_admission,
            manifest,
            world,
            viewer,
            xmap15_selection,
            localisation_language,
            delta_seconds,
            transport_discontinuity,
            day_night_enabled,
            gui_scale,
        );
        self.owned_gfx = Some(gfx);
        let outcome = outcome.and_then(|mut receipt| {
            receipt.presentation_status = Xmap22PresentationStatus::OffscreenReadbackMeasured;
            receipt.presented_to_product_target = true;
            let receipt = receipt.seal()?;
            self.finish_window_present(crate::d3d::PresentOutcome::Presented)?;
            Ok(receipt)
        });
        if outcome.is_err() && self.has_pending_window_present() {
            self.finish_window_present(crate::d3d::PresentOutcome::Skipped)?;
        }
        outcome
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_on_target(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        manifest: &replay_core::ContentManifest,
        world: &WorldFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        xmap15_selection: &map_viewer::Xmap15ViewerSelection,
        localisation_language: &str,
        delta_seconds: f32,
        transport_discontinuity: bool,
        day_night_enabled: bool,
        gui_scale: f32,
    ) -> Result<ProductMapCompositionReceipt> {
        if self.pending_presentation_checkpoint.is_some() {
            return Err("XMAP10 previous composition has no Present disposition".into());
        }
        self.pending_presentation_checkpoint = Some(ProductMapPresentationCheckpoint {
            presentation: self.presentation.clone(),
            map_label_alpha: self.xmap10.map_label_alpha,
        });
        let outcome = self.execute_on_target_candidate(
            gfx,
            resolver,
            content_admission,
            manifest,
            world,
            viewer,
            xmap15_selection,
            localisation_language,
            delta_seconds,
            transport_discontinuity,
            day_night_enabled,
            gui_scale,
        );
        if outcome.is_err() {
            let checkpoint = self
                .pending_presentation_checkpoint
                .take()
                .expect("candidate execution installed its Present checkpoint");
            reject_product_map_presentation(&mut self.presentation, &mut self.xmap10, checkpoint);
        }
        outcome
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_on_target_candidate(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        manifest: &replay_core::ContentManifest,
        world: &WorldFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        xmap15_selection: &map_viewer::Xmap15ViewerSelection,
        localisation_language: &str,
        delta_seconds: f32,
        transport_discontinuity: bool,
        day_night_enabled: bool,
        gui_scale: f32,
    ) -> Result<ProductMapCompositionReceipt> {
        let xmap07_ingress = product_xmap07_shipping_ingress(self.shipping_route, world)?;
        viewer.validate()?;
        let target_device_identity = gfx.device.as_raw() as usize;
        match self.target_device_identity {
            Some(bound) if bound != target_device_identity => {
                return Err(
                    "XMAP22 composition session cannot migrate between D3D11 devices".into(),
                )
            }
            Some(_) => {}
            None => self.target_device_identity = Some(target_device_identity),
        }
        if gfx.width != self.viewport_width
            || gfx.height != self.viewport_height
            || viewer.viewport_width != gfx.width
            || viewer.viewport_height != gfx.height
        {
            return Err("XMAP22 viewer viewport differs from the compositor target".into());
        }
        let camera_state = viewer
            .camera
            .clone()
            .ok_or("XMAP22 product composition has no viewer-owned camera")?;
        let camera = Camera::from_viewer_state(camera_state)?;
        let xmap12_inputs = crate::validate::authenticated_xmap12_frame_inputs(world)?;
        let xmap15_viewer_selection_sha256 =
            snapshot::sha256::Sha256::hex(&serde_json::to_vec(xmap15_selection)?);
        let prepared = self.foundation.prepare_with_xmap12_and_xmap15_inputs(
            world,
            viewer,
            manifest,
            xmap12_inputs.building_visual.as_ref(),
            xmap12_inputs.fixed_runtime.as_ref(),
            xmap15_selection,
        )?;
        if prepared.water.snow_mud_authority_sha256 != xmap07_ingress.authority_sha256
            || prepared.water.hour_index != xmap07_ingress.displayed_hour
        {
            return Err(format!(
                "XMAP07 {:?} shipping ingress differs from the prepared water authority",
                xmap07_ingress.route
            )
            .into());
        }
        require_current_camera_foundation(&camera, &prepared.envelope)?;
        require_current_terrain_foundation(&prepared)?;
        require_current_political_foundation(&prepared)?;
        require_current_lighting_foundation(&camera, &prepared)?;
        require_current_xmap12_foundation(&prepared)?;
        let tree_schedule = self
            .schedule
            .require(
                map_viewer::XMAP11_TREE_STAGE_NAME,
                map_viewer::XMAP11_TREE_OPERATION_NAME,
            )?
            .clone();
        require_current_trees_foundation(&prepared, &tree_schedule)?;
        require_current_weather_foundation(&prepared)?;
        require_current_water_foundation(&prepared)?;
        require_current_thin_borders_foundation(&prepared)?;
        require_current_rivers_straits_foundation(&prepared)?;
        require_current_units_foundation(&prepared)?;
        require_current_orders_foundation(&prepared)?;
        require_current_combat_foundation(&prepared)?;

        self.terrain.retain_frame(gfx, resolver, &prepared)?;
        self.political.retain_frame(gfx, &prepared)?;
        self.trees
            .retain_frame(gfx, resolver, content_admission, &prepared)?;
        self.xmap12.retain_owned_resources(
            gfx,
            resolver,
            content_admission,
            &prepared,
            self.map_width,
            self.map_height,
        )?;
        self.xmap13_units.retain_owned_resources(
            gfx,
            resolver,
            content_admission,
            manifest,
            &prepared,
            self.texture_quality,
            self.map_width,
            self.map_height,
        )?;
        self.lighting.retain_frame(gfx, &camera, &prepared)?;
        self.water.retain_frame(gfx, resolver, &prepared)?;
        self.thin_borders.retain_frame(gfx, resolver, &prepared)?;
        self.rivers_straits.retain_frame(gfx, resolver, &prepared)?;
        self.terrain
            .bind_political_dependencies(gfx, &self.political)?;
        self.terrain.bind_water_dependencies(gfx, &self.water)?;
        self.orders.retain_frame(gfx, resolver, &prepared)?;
        let rail_present = self.rail.retain_frame(gfx, resolver, &prepared)?.is_some();
        if rail_present && viewer.observer_country_id.is_some() {
            let mode_provider = ProductXmap18HubMode {
                map_modes: &prepared.map_modes,
            };
            let selection_provider = map_viewer::D18Xmap16HubViewerSelectionProvider {
                world_frame: world,
                mode_provider: &mode_provider,
            };
            self.rail.retain_hub_frame(
                gfx,
                resolver,
                &prepared,
                viewer,
                &selection_provider,
                gui_scale,
            )?;
        } else {
            self.rail.clear_hub_frame();
        }
        self.combat.update(prepared.combat.clone())?;
        if self.furniture.is_none() {
            self.furniture = Some(FurnitureRenderPass::new(
                gfx,
                resolver,
                localisation_language,
            )?);
        }

        let target_plan =
            map_viewer::Xmap10TargetPlan::new(viewer.viewport_width, viewer.viewport_height)?;
        let main_scene =
            map_viewer::production_main_scene_inputs_from_viewer(viewer, &target_plan)?;
        let presentation_step = map_viewer::Xmap10PresentationRecurrenceStep::from_world_frame(
            world,
            &prepared.terrain.replay_frame_identity_sha256,
            delta_seconds,
            day_night_enabled,
        )?;
        let mut presentation_candidate = self.presentation.clone();
        let presentation_patch = xmap10_apply_bounded_presentation_step(
            &mut presentation_candidate,
            &presentation_step,
            &main_scene.desc,
            transport_discontinuity,
        )?;
        let map_clock = map_viewer::Xmap05AuthenticatedMapClock::from_viewer_presentation(
            prepared.envelope.hour_index,
            viewer,
            &presentation_candidate,
        )?;
        let political_patch = prepared.map_modes.render_political.scene_cb0_patch(
            viewer
                .camera
                .as_ref()
                .ok_or("XMAP22 viewer camera disappeared")?,
            &map_clock,
        )?;
        unsafe {
            self.xmap10.execute_water_frame(
                gfx,
                resolver,
                content_admission,
                manifest,
                world,
                localisation_language,
                delta_seconds,
                transport_discontinuity,
                &prepared,
                viewer,
                &political_patch,
                &presentation_patch,
                main_scene.desc,
                main_scene.color,
                main_scene.depth,
                main_scene.producer_sha256,
                &mut self.terrain,
                &self.political,
                &mut self.lighting,
                &mut self.trees,
                &tree_schedule,
                &mut self.xmap12,
                &mut self.xmap13_units,
                &mut self.water,
                &mut self.thin_borders,
                &mut self.rivers_straits,
                &mut self.orders,
                &mut self.rail,
                self.furniture
                    .as_mut()
                    .ok_or("XMAP22 furniture renderer disappeared")?,
            )?;
        }

        if !self.xmap12.execution_complete() {
            return Err(format!(
                "XMAP22 complete frame has no XMAP12 execution: {}",
                self.xmap12
                    .unavailable_reason()
                    .unwrap_or("missing without typed reason")
            )
            .into());
        }
        let unit_receipt = self.xmap13_units.completed_receipt().ok_or_else(|| {
            format!(
                "XMAP22 complete frame has no XMAP13 unit execution: {}",
                self.xmap13_units
                    .unavailable_reason()
                    .unwrap_or("missing without typed reason")
            )
        })?;
        let unit_receipt_sha256 = unit_receipt.packet_draw_receipt_sha256.clone();
        let counter_target_sha256 = xmap13_counter_target_sha256(gfx)?;
        let counter_receipt = self.xmap13_counters.execute(
            gfx,
            prepared
                .units
                .as_ref()
                .ok_or("XMAP22 XMAP13 payload disappeared")?,
            viewer,
            unit_receipt,
            &counter_target_sha256,
        )?;
        let counter_receipt_sha256 = counter_receipt.packet_draw_receipt_sha256.clone();
        let viewer_state_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(viewer)?);
        let combat_target_sha256 = xmap14_combat_target_sha256(gfx)?;
        // THE COMBAT LAYER MAY BE EXCLUDED AND NAMED. When XMAP14's foundation
        // row is Unavailable there is no execution receipt to seal, and there
        // must not be an invented one; what the frame carries instead is the
        // gap's own witness, so the receipt says WHY the layer is missing
        // rather than merely lacking it. Requiring a receipt here undid the
        // whole exclusion downstream (Codex review of batch 3) -- and it is the
        // same shape XMAP16's hub bucket already handles below with its
        // no-observer witness.
        let combat_unavailable_witness = prepared
            .envelope
            .families
            .iter()
            .find(|row| row.family == map_viewer::MapVisualFamily::CombatMarkers)
            .and_then(|row| match &row.disposition {
                map_viewer::FamilyDisposition::Unavailable { gap } => {
                    Some(gap.witness_sha256.clone())
                }
                _ => None,
            });
        let (combat_receipt, hub_receipt) = unsafe {
            let xmap10 = &mut self.xmap10;
            let combat = &mut self.combat;
            let rail = &mut self.rail;
            let furniture = self
                .furniture
                .as_mut()
                .ok_or("XMAP22 furniture renderer disappeared before bucket execution")?;
            xmap10.execute_map_icon_buckets(
                gfx,
                resolver,
                furniture,
                &counter_receipt_sha256,
                || {
                    let receipt = combat.execute(gfx, &combat_target_sha256)?.cloned();
                    let witness = xmap22_execution_witness_or_named_gap(
                        receipt.as_ref().map(|value| value.deterministic_sha256.as_str()),
                        combat_unavailable_witness.as_deref(),
                        "XMAP14",
                    )?;
                    Ok((receipt, witness))
                },
                || {
                    let receipt = rail.execute_hubs(gfx)?;
                    let no_observer_witness =
                        snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(
                            "xmap16.hub.no-observer-country.v1",
                            viewer_state_sha256.as_str(),
                        ))?);
                    let witness = receipt.as_ref().map_or(no_observer_witness, |value| {
                        value.deterministic_sha256.clone()
                    });
                    Ok((receipt, witness))
                },
            )?
        };
        // The named gap's witness stands in for an execution that correctly did
        // not happen. The family census row is `Unavailable` and carries the
        // same witness, and the receipt validator requires the two to agree.
        let combat_receipt_sha256 = xmap22_execution_witness_or_named_gap(
            combat_receipt.as_ref().map(|value| value.deterministic_sha256.as_str()),
            combat_unavailable_witness.as_deref(),
            "XMAP14",
        )?;
        if !self.xmap10.map_label_execution_complete()
            || !self.xmap10.map_furniture_execution_complete()
            || !self.orders.execution_complete()
            || !self.rail.execution_complete()
        {
            return Err("XMAP22 complete frame did not execute every packet compositor".into());
        }
        let xmap16_hub = match (viewer.observer_country_id.as_ref(), hub_receipt) {
            (Some(_), Some(receipt)) => Xmap16HubCompositionDisposition::Present {
                execution_receipt_sha256: receipt.deterministic_sha256,
            },
            (None, None) => Xmap16HubCompositionDisposition::NoObserverCountry {
                viewer_state_sha256: viewer_state_sha256.clone(),
            },
            (Some(_), None) => {
                return Err("XMAP22 selected-country frame skipped XMAP16 hub execution".into())
            }
            (None, Some(_)) => {
                return Err("XMAP22 no-observer frame unexpectedly executed XMAP16 hubs".into())
            }
        };
        let xmap12_execution_sha256 = self
            .xmap12
            .completed_receipt()
            .ok_or("XMAP22 complete frame lost its XMAP12 execution receipt")?
            .deterministic_sha256
            .clone();
        let xmap11_tree_execution_sha256 = self
            .trees
            .completed_receipt()
            .ok_or("XMAP22 complete frame lost its XMAP11 tree execution receipt")?
            .deterministic_sha256
            .clone();
        let xmap15_execution_sha256 = self
            .orders
            .execution_receipt
            .as_ref()
            .ok_or("XMAP22 complete frame lost its XMAP15 execution receipt")?
            .deterministic_sha256
            .clone();
        let (xmap16_rail_execution_sha256, xmap16_supply_execution_sha256s) = self
            .rail
            .execution_receipt_sha256s()
            .map(|(rail, supply)| (rail.to_owned(), supply.to_vec()))?;
        let supply_view_active = prepared.map_modes.xmap16_supply_view_active()?;
        let xmap16_supply = if supply_view_active {
            if xmap16_supply_execution_sha256s.len() != 2 {
                return Err(
                    "XMAP22 supply map mode did not execute both XMAP16 supply passes".into(),
                );
            }
            Xmap16SupplyCompositionDisposition::Present {
                normal_receipt_sha256: xmap16_supply_execution_sha256s[0].clone(),
                see_through_receipt_sha256: xmap16_supply_execution_sha256s[1].clone(),
            }
        } else {
            if !xmap16_supply_execution_sha256s.is_empty() {
                return Err(
                    "XMAP22 non-supply map mode unexpectedly executed XMAP16 supply passes".into(),
                );
            }
            Xmap16SupplyCompositionDisposition::MapModeInactive {
                prepared_map_modes_sha256: prepared.map_modes.deterministic_sha256.clone(),
            }
        };
        let xmap17_execution_sha256 = self
            .xmap10
            .map_label_draw
            .as_ref()
            .ok_or("XMAP22 complete frame lost its XMAP17 execution receipt")?
            .packet_sha256
            .clone();
        let xmap21_execution_sha256 = self
            .xmap10
            .xmap21_execution
            .as_ref()
            .ok_or("XMAP22 complete frame lost its XMAP21 execution receipt")?
            .deterministic_sha256
            .clone();
        let xmap20 = self
            .xmap10
            .xmap20_effects
            .as_ref()
            .ok_or("XMAP22 complete frame has no XMAP20 execution status")?;
        xmap20.validate()?;
        let xmap20_scene_receipt_sha256 = xmap20.scene_receipt_sha256().to_owned();
        let completed_postfx = self
            .xmap10
            .completed_postfx
            .as_ref()
            .ok_or("XMAP22 complete frame has no XMAP10 post-FX receipt")?;
        completed_postfx.validate()?;
        let xmap10_postfx_receipt_sha256 = completed_postfx
            .postfx_frame
            .packet_frame_receipt_sha256
            .clone();
        if self.gpu_completion_query.is_none() {
            self.gpu_completion_query = Some(gfx.create_gpu_completion_query()?);
        }
        self.gpu_completion_query
            .as_ref()
            .ok_or("XMAP22 GPU completion query disappeared")?
            .wait(gfx)?;
        let pixels = gfx.read_target()?;
        let final_pixels_sha256 = snapshot::sha256::Sha256::hex(&pixels);
        let foundation_envelope_sha256 = prepared
            .envelope
            .deterministic_sha256()
            .map_err(|error| format!("XMAP22 foundation digest: {error}"))?;
        if !prepared.envelope.is_complete() {
            return Err("XMAP22 cannot seal a composition with an incomplete foundation".into());
        }
        let xmap04_terrain_execution_sha256 = self
            .xmap10
            .terrain_draw
            .as_ref()
            .ok_or("XMAP22 lost the XMAP04 terrain draw receipt")?
            .deterministic_sha256
            .clone();
        let xmap05_resource_execution_sha256 = self
            .political
            .handoff
            .as_ref()
            .ok_or("XMAP22 lost the XMAP05 resource handoff")?
            .receipt()
            .deterministic_sha256
            .clone();
        let xmap06_execution_sha256 = self.thin_borders.completed_scene_receipt_sha256()?;
        let xmap07_execution_sha256 = self.water.completed_scene_receipt_sha256()?;
        let xmap08_execution_sha256 = self
            .rivers_straits
            .completed_scene_receipt_sha256()?
            .to_owned();
        let xmap09_execution_sha256 = self
            .lighting
            .handoff
            .as_ref()
            .ok_or("XMAP22 lost the XMAP09 light/depth execution")?
            .receipt()
            .deterministic_sha256
            .clone();
        let xmap13_execution_sha256 = xmap22_family_execution_sha256(
            map_viewer::MapVisualFamily::UnitsCounters,
            &[&unit_receipt_sha256, &counter_receipt_sha256],
        )?;
        let xmap16_supply_disposition_sha256 =
            snapshot::sha256::Sha256::hex(&serde_json::to_vec(&xmap16_supply)?);
        let xmap16_hub_disposition_sha256 =
            snapshot::sha256::Sha256::hex(&serde_json::to_vec(&xmap16_hub)?);
        let xmap16_execution_sha256 = xmap22_family_execution_sha256(
            map_viewer::MapVisualFamily::RailSupply,
            &[
                &xmap16_rail_execution_sha256,
                &xmap16_supply_disposition_sha256,
                &xmap16_hub_disposition_sha256,
            ],
        )?;
        let weather_execution_sha256 = prepared
            .weather
            .as_ref()
            .ok_or("XMAP22 complete frame lost its XMAP19 weather execution provider")?
            .deterministic_sha256
            .clone();
        let mut family_execution_witnesses = BTreeMap::new();
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::CameraView,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::CameraView,
                &[&viewer_state_sha256, &xmap04_terrain_execution_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::Terrain,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::Terrain,
                &[&xmap04_terrain_execution_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::PoliticalFillGradients,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::PoliticalFillGradients,
                &[
                    &xmap05_resource_execution_sha256,
                    &xmap04_terrain_execution_sha256,
                ],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::FogOfWar,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::FogOfWar,
                &[
                    &xmap05_resource_execution_sha256,
                    &xmap04_terrain_execution_sha256,
                ],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::ThinBorders,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::ThinBorders,
                &[&xmap06_execution_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::WaterIce,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::WaterIce,
                &[
                    &xmap07_execution_sha256,
                    &prepared.water.snow_mud_authority_sha256,
                ],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::RiversStraits,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::RiversStraits,
                &[&xmap08_execution_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::LightingShadows,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::LightingShadows,
                &[&xmap09_execution_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::GradingPostFx,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::GradingPostFx,
                &[&xmap10_postfx_receipt_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::Trees,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::Trees,
                &[&xmap11_tree_execution_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::CitiesBuildingsMapObjects,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::CitiesBuildingsMapObjects,
                &[&xmap12_execution_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::UnitsCounters,
            xmap13_execution_sha256,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::CombatMarkers,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::CombatMarkers,
                &[&combat_receipt_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::OrdersFrontsArrows,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::OrdersFrontsArrows,
                &[&xmap15_execution_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::RailSupply,
            xmap16_execution_sha256,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::LabelsMapText,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::LabelsMapText,
                &[&xmap17_execution_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::MapModes,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::MapModes,
                &[
                    &prepared.map_modes.deterministic_sha256,
                    &xmap04_terrain_execution_sha256,
                ],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::WeatherAtmosphere,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::WeatherAtmosphere,
                &[&weather_execution_sha256, &xmap20_scene_receipt_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::ParticlesEffects,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::ParticlesEffects,
                &[&xmap20_scene_receipt_sha256],
            )?,
        );
        family_execution_witnesses.insert(
            map_viewer::MapVisualFamily::MapFurnitureIcons,
            xmap22_family_execution_sha256(
                map_viewer::MapVisualFamily::MapFurnitureIcons,
                &[&xmap21_execution_sha256],
            )?,
        );
        let mandatory_family_execution_census =
            build_xmap22_family_execution_census(&prepared.envelope, &family_execution_witnesses)?;
        let mandatory_family_execution_census_sha256 =
            snapshot::sha256::Sha256::hex(&serde_json::to_vec(&mandatory_family_execution_census)?);
        let receipt = ProductMapCompositionReceipt {
            schema: "hoi4.replay-map-composition.xmap22.v4".into(),
            hour_index: prepared.envelope.hour_index,
            viewport_width: viewer.viewport_width,
            viewport_height: viewer.viewport_height,
            content_manifest_sha256: prepared.envelope.content_manifest_sha256.clone(),
            product_modset_sha256: prepared.envelope.product_modset_sha256.clone(),
            effective_vfs_manifest_sha256: prepared.envelope.effective_vfs_manifest_sha256.clone(),
            replay_frame_identity_sha256: prepared.terrain.replay_frame_identity_sha256.clone(),
            viewer_state_sha256,
            xmap15_viewer_selection_sha256,
            assigned_stage_topology_sha256: self.topology_sha256.clone(),
            assigned_stage_schedule_sha256: self.schedule_sha256.clone(),
            foundation_envelope_sha256,
            foundation_content_operation_sha256: prepared
                .envelope
                .foundation_content_operation_sha256
                .clone(),
            xmap04_terrain_execution_sha256,
            xmap05_resource_execution_sha256,
            xmap06_thin_borders_execution_sha256: xmap06_execution_sha256,
            xmap07_water_execution_sha256: xmap07_execution_sha256,
            xmap07_snow_mud_authority_sha256: prepared.water.snow_mud_authority_sha256.clone(),
            xmap08_rivers_straits_execution_sha256: xmap08_execution_sha256,
            xmap09_lighting_execution_sha256: xmap09_execution_sha256,
            xmap10_postfx_receipt_sha256,
            xmap20_scene_receipt_sha256,
            xmap11_tree_execution_sha256,
            xmap12_execution_sha256,
            xmap13_unit_execution_sha256: unit_receipt_sha256,
            xmap13_counter_execution_sha256: counter_receipt_sha256,
            xmap14_execution_sha256: combat_receipt_sha256,
            xmap15_execution_sha256,
            xmap16_rail_execution_sha256,
            prepared_map_modes_sha256: prepared.map_modes.deterministic_sha256.clone(),
            prepared_map_mode_composition: prepared.map_modes.composition.token().into(),
            xmap16_supply,
            xmap17_execution_sha256,
            xmap19_weather_execution_sha256: weather_execution_sha256,
            xmap21_execution_sha256,
            xmap16_hub,
            mandatory_family_count: u32::try_from(map_viewer::MapVisualFamily::ALL.len())?,
            mandatory_family_execution_census,
            mandatory_family_execution_census_sha256,
            foundation_complete: true,
            execution_complete: true,
            final_pixels_measured: true,
            presented_to_product_target: false,
            presentation_status: Xmap22PresentationStatus::WindowPendingPresent,
            final_target_sha256: gfx.target_resource_identity_sha256()?.to_owned(),
            final_pixels_sha256,
            deterministic_sha256: String::new(),
        }
        .seal()?;
        self.presentation = presentation_candidate;
        self.last_prepared = Some(prepared);
        Ok(receipt)
    }

    pub(crate) fn last_prepared(&self) -> Option<&map_viewer::PreparedFoundationFrame> {
        self.last_prepared.as_ref()
    }

    pub(crate) fn map_furniture_packet(&self) -> Option<&map_viewer::PreparedMapFurniturePacket> {
        self.xmap10.map_furniture_packet()
    }

    pub(crate) fn finish_window_present(
        &mut self,
        outcome: crate::d3d::PresentOutcome,
    ) -> Result<()> {
        let checkpoint = self
            .pending_presentation_checkpoint
            .take()
            .ok_or("XMAP10 composition has no pending Present disposition")?;
        if outcome != crate::d3d::PresentOutcome::Presented {
            reject_product_map_presentation(&mut self.presentation, &mut self.xmap10, checkpoint);
        }
        Ok(())
    }

    fn has_pending_window_present(&self) -> bool {
        self.pending_presentation_checkpoint.is_some()
    }
}

impl ProductProductionD3dRuntime {
    pub(crate) fn execute(
        &mut self,
        gfx: &Gfx,
        resolver: &GameFileResolver,
        resources: &std::collections::BTreeMap<String, map_viewer::ProductionMapResource>,
        frame: &map_viewer::PreparedProductionD3dFrame,
    ) -> Result<ProductProductionD3dOutcome> {
        self.execute_inner(gfx, resolver, resources, frame, false)
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

fn transport_from_provider_index(provider: &dyn OwnerProvider) -> Result<Transport> {
    Transport::with_lazy_dates(provider.hours().to_vec()).map_err(|error| error.into())
}

fn reject_production_recording_debug_surfaces(
    production_recording: bool,
    has_bundle: bool,
    has_capture: bool,
) -> Result<()> {
    if !production_recording {
        return Ok(());
    }
    use crate::blockers::{reason, BlockerId};
    if has_bundle {
        return Err(reason(BlockerId::RecordingBundle).into());
    }
    if has_capture {
        return Err(reason(BlockerId::RecordingCapture).into());
    }
    Ok(())
}

fn current_camera_viewer_state(
    camera: &Camera,
    observer_country_id: Option<&str>,
    map_mode: &map_viewer::ViewerMapModeSelection,
    map_mode_selection_epoch: u64,
    selected_object_ids: &[String],
    animation_time: f32,
    frame_delta_seconds: f32,
) -> Result<map_viewer::ViewerOwnedMapState> {
    let camera_state = camera
        .viewer_state()
        .ok_or("XMAP03 product route has no viewer-owned camera state")?;
    Ok(map_viewer::ViewerOwnedMapState {
        viewport_width: camera_state.viewport_width,
        viewport_height: camera_state.viewport_height,
        camera: Some(camera_state.clone()),
        observer_country_id: observer_country_id.map(str::to_string),
        map_mode_id: map_mode.viewer_token(),
        map_mode_selection_epoch,
        selected_object_ids: selected_object_ids.to_vec(),
        input_revision: camera_state.input_revision,
        animation_time_f32_bits: animation_time.to_bits(),
        frame_delta_seconds_f32_bits: frame_delta_seconds.to_bits(),
    })
}

fn require_current_camera_foundation(
    camera: &Camera,
    envelope: &map_viewer::FoundationRenderEnvelope,
) -> Result<()> {
    let camera_state = camera
        .viewer_state()
        .ok_or("XMAP03 product route has no viewer-owned camera state")?;
    let camera_row = envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::CameraView)
        .ok_or("XMAP03 foundation envelope omitted CameraView")?;
    match &camera_row.disposition {
        map_viewer::FamilyDisposition::Present { payload_sha256, .. }
            if payload_sha256 == &camera_state.deterministic_sha256 =>
        {
            Ok(())
        }
        _ => Err("XMAP03 foundation disposition drifted from the live viewer camera".into()),
    }
}

fn require_current_terrain_foundation(
    prepared: &map_viewer::PreparedFoundationFrame,
) -> Result<()> {
    let terrain_row = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::Terrain)
        .ok_or("XMAP04 foundation envelope omitted Terrain")?;
    match &terrain_row.disposition {
        map_viewer::FamilyDisposition::Present { payload_sha256, .. }
            if payload_sha256 == &prepared.terrain.deterministic_sha256
                && !prepared.terrain.draws.is_empty() =>
        {
            Ok(())
        }
        _ => Err("XMAP04 foundation disposition has no matching terrain draw payload".into()),
    }
}

fn require_current_political_foundation(
    prepared: &map_viewer::PreparedFoundationFrame,
) -> Result<()> {
    prepared.political.validate()?;
    let political = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::PoliticalFillGradients)
        .ok_or("XMAP05 foundation envelope omitted PoliticalFillGradients")?;
    let fow = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::FogOfWar)
        .ok_or("XMAP05 foundation envelope omitted FogOfWar")?;
    if matches!(
        &political.disposition,
        map_viewer::FamilyDisposition::Present { payload_sha256, .. }
            if payload_sha256 == &prepared.political.political_family_sha256
    ) && matches!(
        &fow.disposition,
        map_viewer::FamilyDisposition::Present { payload_sha256, .. }
            if payload_sha256 == &prepared.political.fow_family_sha256
    ) {
        Ok(())
    } else {
        Err("XMAP05 foundation dispositions drifted from the political/FoW payload".into())
    }
}

fn require_current_lighting_foundation<'a>(
    camera: &Camera,
    prepared: &'a map_viewer::PreparedFoundationFrame,
) -> Result<Option<&'a map_viewer::PreparedXmap09LightingFrame>> {
    let camera_state = camera
        .viewer_state()
        .ok_or("XMAP09 product route has no viewer-owned camera state")?;
    let row = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::LightingShadows)
        .ok_or("XMAP09 foundation envelope omitted LightingShadows")?;
    match (&row.disposition, prepared.lighting.as_ref()) {
        (map_viewer::FamilyDisposition::Present { payload_sha256, .. }, Some(lighting))
            if payload_sha256 == &lighting.deterministic_sha256
                && lighting.camera_frame_sha256 == camera_state.deterministic_sha256
                && lighting.replay_frame_identity_sha256
                    == prepared.terrain.replay_frame_identity_sha256 =>
        {
            lighting.validate()?;
            Ok(Some(lighting))
        }
        (map_viewer::FamilyDisposition::Unavailable { .. }, None) => Ok(None),
        _ => Err(
            "XMAP09 foundation disposition/payload/camera/replay identity is not current and exact"
                .into(),
        ),
    }
}

fn require_current_xmap12_foundation(
    prepared: &map_viewer::PreparedFoundationFrame,
) -> Result<Option<&map_viewer::PreparedXmap12RenderFrame>> {
    let row = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::CitiesBuildingsMapObjects)
        .ok_or("XMAP12 foundation envelope omitted CitiesBuildingsMapObjects")?;
    match (&row.disposition, prepared.xmap12.as_ref()) {
        (map_viewer::FamilyDisposition::Present { payload_sha256, .. }, Some(frame))
            if payload_sha256 == &frame.deterministic_sha256
                && frame.replay_frame_identity_sha256
                    == prepared.terrain.replay_frame_identity_sha256 =>
        {
            frame.validate()?;
            Ok(Some(frame))
        }
        (map_viewer::FamilyDisposition::Unavailable { gap }, None)
            if gap.gap_id == "XMAP12.exact_per_frame_inputs_unavailable"
                && !gap.reason.trim().is_empty() =>
        {
            Ok(None)
        }
        _ => Err(
            "XMAP12 foundation disposition/payload/replay identity is not current and exact".into(),
        ),
    }
}

fn require_current_trees_foundation<'a>(
    prepared: &'a map_viewer::PreparedFoundationFrame,
    schedule: &map_viewer::AssignedStage,
) -> Result<&'a map_viewer::PreparedTreeFrame> {
    let row = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::Trees)
        .ok_or("XMAP11 foundation envelope omitted Trees")?;
    let frame = prepared
        .trees
        .as_ref()
        .ok_or("XMAP11 foundation omitted its prepared tree frame")?;
    frame.validate()?;
    let disposition_matches =
        row.disposition == map_viewer::xmap11_foundation_disposition(frame, schedule)?;
    if disposition_matches
        && frame.hour_index == prepared.envelope.hour_index
        && frame.replay_frame_identity_sha256 == prepared.terrain.replay_frame_identity_sha256
        && frame.camera_frame_sha256 == prepared.terrain.camera_frame_sha256
        && frame.map_copy_receipt_set_sha256 == prepared.terrain.map_copy_receipt_set_sha256()?
    {
        Ok(frame)
    } else {
        Err(
            "XMAP11 foundation disposition/payload/replay/camera/map-copy identity is not current and exact"
                .into(),
        )
    }
}

fn require_current_weather_foundation(
    prepared: &map_viewer::PreparedFoundationFrame,
) -> Result<()> {
    let row = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::WeatherAtmosphere)
        .ok_or("XMAP19 foundation envelope omitted WeatherAtmosphere")?;
    match (&prepared.weather, &row.disposition) {
        (Some(weather), map_viewer::FamilyDisposition::Present { payload_sha256, .. })
            if payload_sha256 == &weather.deterministic_sha256 =>
        {
            weather.validate()
        }
        (None, map_viewer::FamilyDisposition::Unavailable { .. }) => Ok(()),
        _ => Err(
            "XMAP19 foundation disposition drifted from its prepared/typed-unavailable payload"
                .into(),
        ),
    }
}

fn require_current_water_foundation(prepared: &map_viewer::PreparedFoundationFrame) -> Result<()> {
    prepared.water.validate()?;
    let water = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::WaterIce)
        .ok_or("XMAP07 foundation envelope omitted WaterIce")?;
    if matches!(
        &water.disposition,
        map_viewer::FamilyDisposition::Present { payload_sha256, .. }
            if payload_sha256 == &prepared.water.deterministic_sha256
    ) {
        Ok(())
    } else {
        Err("XMAP07 foundation disposition drifted from the water/ice payload".into())
    }
}

fn require_current_thin_borders_foundation(
    prepared: &map_viewer::PreparedFoundationFrame,
) -> Result<Option<&map_viewer::PreparedThinBorderFrame>> {
    let row = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::ThinBorders)
        .ok_or("XMAP06 foundation envelope omitted ThinBorders")?;
    match (&row.disposition, prepared.thin_borders.as_ref()) {
        (map_viewer::FamilyDisposition::Present { payload_sha256, .. }, Some(frame))
            if payload_sha256 == &frame.deterministic_sha256
                && frame.runtime_partition.is_some()
                && frame.hour_index == prepared.envelope.hour_index
                && frame.map_width == prepared.terrain.map_width
                && frame.map_height == prepared.terrain.map_height =>
        {
            frame.validate()?;
            Ok(Some(frame))
        }
        (map_viewer::FamilyDisposition::Unavailable { gap }, Some(frame))
            if gap.gap_id == map_viewer::XMAP06_RUNTIME_BUCKET_GAP_ID
                && frame.runtime_partition.is_none() =>
        {
            frame.validate()?;
            Ok(None)
        }
        _ => Err(
            "XMAP06 foundation disposition/payload/runtime/map identity is not current and exact"
                .into(),
        ),
    }
}

fn require_current_rivers_straits_foundation(
    prepared: &map_viewer::PreparedFoundationFrame,
) -> Result<Option<&map_viewer::PreparedRiversStraitsFrame>> {
    let row = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::RiversStraits)
        .ok_or("XMAP08 foundation envelope omitted RiversStraits")?;
    match (&row.disposition, prepared.rivers_straits.as_ref()) {
        (map_viewer::FamilyDisposition::Present { payload_sha256, .. }, Some(frame))
            if payload_sha256 == &frame.deterministic_sha256
                && frame.camera_view_sha256 == prepared.terrain.camera_frame_sha256
                && frame.map_width == prepared.terrain.map_width
                && frame.map_height == prepared.terrain.map_height =>
        {
            frame.validate()?;
            Ok(Some(frame))
        }
        (map_viewer::FamilyDisposition::Unavailable { .. }, None) => Ok(None),
        _ => Err(
            "XMAP08 foundation disposition/payload/camera/map identity is not current and exact"
                .into(),
        ),
    }
}

fn require_current_units_foundation(prepared: &map_viewer::PreparedFoundationFrame) -> Result<()> {
    let row = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::UnitsCounters)
        .ok_or("XMAP13 foundation envelope omitted UnitsCounters")?;
    match (&row.disposition, prepared.units.as_ref()) {
        (map_viewer::FamilyDisposition::Present { payload_sha256, .. }, Some(units))
            if payload_sha256 == &units.deterministic_sha256 =>
        {
            units.validate()
        }
        (map_viewer::FamilyDisposition::Unavailable { .. }, None) => Ok(()),
        _ => Err("XMAP13 foundation disposition drifted from its prepared unit payload".into()),
    }
}

fn require_current_combat_foundation(prepared: &map_viewer::PreparedFoundationFrame) -> Result<()> {
    let row = prepared
        .envelope
        .families
        .iter()
        .find(|row| row.family == map_viewer::MapVisualFamily::CombatMarkers)
        .ok_or("XMAP14 foundation envelope omitted CombatMarkers")?;
    match (&row.disposition, prepared.combat.as_ref()) {
        (map_viewer::FamilyDisposition::Present { payload_sha256, .. }, Some(combat))
            if payload_sha256 == &combat.deterministic_sha256 && !combat.indicators.is_empty() =>
        {
            combat.validate()
        }
        (
            map_viewer::FamilyDisposition::AuthenticatedEmpty {
                enumerator_witness_sha256,
                ..
            },
            Some(combat),
        ) if combat.indicators.is_empty()
            && enumerator_witness_sha256 == &combat.enumerator_witness_sha256 =>
        {
            combat.validate()
        }
        (map_viewer::FamilyDisposition::Unavailable { .. }, None) => Ok(()),
        _ => Err("XMAP14 foundation disposition drifted from its prepared combat payload".into()),
    }
}

struct ProductOrdersHandoffRuntime {
    native: Option<map_viewer::NativeXmap15Handoff>,
    lifecycle: map_viewer::Xmap15LifecycleState,
    lifecycle_update: Option<map_viewer::Xmap15LifecycleUpdate>,
    execution_receipt: Option<map_viewer::Xmap15NativeExecutionReceipt>,
    unavailable_reason: Option<String>,
}

impl Default for ProductOrdersHandoffRuntime {
    fn default() -> Self {
        Self {
            native: None,
            lifecycle: map_viewer::Xmap15LifecycleState::default(),
            lifecycle_update: None,
            execution_receipt: None,
            unavailable_reason: Some("XMAP15 orders foundation has not been retained".into()),
        }
    }
}

fn require_current_orders_foundation(
    prepared: &map_viewer::PreparedFoundationFrame,
) -> Result<Option<&map_viewer::PreparedXmap15Frame>> {
    match prepared.xmap15_orders_state()? {
        map_viewer::Xmap15FoundationState::Present(orders)
        | map_viewer::Xmap15FoundationState::AuthenticatedEmpty(orders) => Ok(Some(orders)),
        map_viewer::Xmap15FoundationState::Unavailable(_) => Ok(None),
    }
}

fn xmap13_counter_target_sha256(gfx: &Gfx) -> Result<String> {
    Ok(snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(
        "hoi4.xmap13.counter-target.a729.v1",
        gfx.target_resource_identity_sha256()?,
        gfx.target_allocation_generation(),
        gfx.width,
        gfx.height,
        gfx.target_format(),
    ))?))
}

fn xmap14_combat_target_sha256(gfx: &Gfx) -> Result<String> {
    Ok(snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(
        "hoi4.xmap14.combat-indicator-swapchain-target.a729.v1",
        gfx.target_resource_identity_sha256()?,
        gfx.target_allocation_generation(),
        gfx.width,
        gfx.height,
        gfx.target_format(),
    ))?))
}

fn xmap10_requires_temporal_resource_reset(
    transport_discontinuity: bool,
    target_missing_or_changed: bool,
) -> bool {
    transport_discontinuity || target_missing_or_changed
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProductMapIconPointerHit {
    Furniture(map_viewer::MapFurnitureTarget),
    Combat(map_viewer::Xmap14IndicatorIdentity),
    Unit(map_viewer::Xmap13UnitIdentity),
}

impl ProductMapIconPointerHit {
    fn bucket_ordinal(&self) -> u8 {
        match self {
            Self::Furniture(target) => target.map_icon_bucket_ordinal(),
            Self::Combat(identity) => identity.map_icon_bucket_ordinal(),
            Self::Unit(_) => 1,
        }
    }
}

fn select_topmost_product_map_icon_hit(
    furniture_hits: Vec<map_viewer::MapFurnitureTarget>,
    combat_hit: Option<&map_viewer::Xmap14IndicatorIdentity>,
    unit_hit: Option<&map_viewer::Xmap13UnitIdentity>,
) -> Option<ProductMapIconPointerHit> {
    let candidates = furniture_hits
        .into_iter()
        .map(ProductMapIconPointerHit::Furniture)
        .chain(combat_hit.cloned().map(ProductMapIconPointerHit::Combat))
        .chain(unit_hit.cloned().map(ProductMapIconPointerHit::Unit));
    candidates.fold(None, |front, candidate| match front {
        Some(current) if current.bucket_ordinal() >= candidate.bucket_ordinal() => Some(current),
        _ => Some(candidate),
    })
}

fn xmap10_apply_bounded_presentation_step(
    state: &mut map_viewer::Xmap10ViewerPresentationState,
    step: &map_viewer::Xmap10PresentationRecurrenceStep,
    main_scene: &map_viewer::ProductionMainSceneTargetDesc,
    _transport_discontinuity: bool,
) -> Result<map_viewer::Xmap10PresentationScenePatch> {
    step.validate()?;
    state.advance_step(step, main_scene)
}

#[derive(Clone, Copy, Debug)]
struct PresentedFrameTiming {
    origin: Instant,
    last_presented: Instant,
}

impl PresentedFrameTiming {
    fn new(origin: Instant) -> Self {
        Self {
            origin,
            last_presented: origin,
        }
    }

    fn advance_candidate(&mut self, now: Instant) -> Result<(u64, f64)> {
        let viewer_clock_micros = u64::try_from(now.duration_since(self.origin).as_micros())
            .map_err(|_| "viewer UI monotonic clock exceeds u64 microseconds")?;
        let delta_seconds = (now - self.last_presented).as_secs_f64().min(MAX_FRAME_DT);
        self.last_presented = now;
        Ok((viewer_clock_micros, delta_seconds))
    }
}

#[derive(Clone, Debug)]
struct ReplayWindowEventCheckpoint {
    transport: crate::transport::TransportPresentationCheckpoint,
    pov: PovState,
    pov_picker: PovPicker,
    viewer_map_mode: map_viewer::ViewerMapModeSelection,
    map_mode_selection_epoch: u64,
    camera: Option<Camera>,
    occupation_wheel: OccupationWheelAccumulator,
    character_wheel_remainder: i64,
    occupation_expand_all: bool,
    battleplan_selection: BattleplanSelectionState,
    selected_xmap13_ids: Vec<String>,
    selected_xmap14_id: Option<String>,
    day_night_enabled: bool,
    pan_keys: PanKeys,
    drag: Option<DragState>,
    scrub_drag: bool,
    wheel_remainder_units: i32,
    cursor: Option<(i32, i32)>,
    frame_timing: PresentedFrameTiming,
    map_animation_time: f32,
    occupation_input_region: Option<game_ui::occupation_compositor::OccupationInputRegion>,
    character_input_region: Option<game_ui::character_ui::CharacterInputRegion>,
    character_hit_targets: Vec<game_ui::character_ui::CharacterHitTarget>,
    character_semantic_state: Option<game_ui::character_ui::ObserverCharacterUi>,
    last_product_transport_index: Option<usize>,
}

fn product_map_camera_drag_allowed(
    button: MouseButton,
    xui20_owns_pointer: bool,
    any_other_ui_owns_pointer: bool,
) -> bool {
    matches!(button, MouseButton::Middle | MouseButton::Right)
        && !xui20_owns_pointer
        && !any_other_ui_owns_pointer
}

fn cancel_app_pointer_capture_for_resize<T>(drag: &mut Option<T>, scrub_drag: &mut bool) {
    *drag = None;
    *scrub_drag = false;
}

fn refresh_cached_input_viewport(
    screen_w: &mut f64,
    screen_h: &mut f64,
    width: u32,
    height: u32,
) {
    *screen_w = f64::from(width);
    *screen_h = f64::from(height);
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
    reject_production_recording_debug_surfaces(
        production_recording,
        bundle.is_some(),
        capture_dir.is_some(),
    )?;
    let (provider, mode_label) = build_provider(bundle.as_ref(), &mode)?;
    let (game_resolver, content_admission) =
        crate::content_compatibility::bind_provider_content(provider.as_ref(), &product_paths)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let content_manifest = provider
        .content_manifest()
        .ok_or("portable playback has no admitted content manifest after binding")?;
    let terrain_lod_options =
        crate::install_paths::playback_terrain_lod_options_from_paths(&product_paths)?;
    let mut transport = transport_from_provider_index(provider.as_ref())?;
    let hours: Vec<i64> = transport.hours().to_vec();
    let initial_frame = provider
        .world_frame_at(hours[0])?
        .ok_or_else(|| map_viewer::refuse_missing_xmap00_authority("world_frame"))?;
    let mut reported_production_data_needs = BTreeSet::new();
    // The scrub engine takes ownership of the frame path from here on: the
    let mut scrub = crate::scrub::ScrubEngine::start(Arc::clone(&provider), hours[0])?;
    let mut last_reported_scrub_refusal: Option<i64> = None;
    let mut pov_picker = provider.pov_picker()?;
    let mut pov = PovState::new();
    match &pov_picker {
        PovPicker::Available(choices) => eprintln!(
            "replay_app: observer country picker: {} d18 countr{} (press Tab to cycle all countries, 1-9 for direct slots, O to request/clear that country's fog)",
            choices.len(),
            if choices.len() == 1 { "y" } else { "ies" }
        ),
        PovPicker::Unavailable(reason) => {
            eprintln!("replay_app: observer country picker unavailable: {reason}")
        }
    }
    let max_state_id = bundle.as_ref().map(MapBundle::max_state_id);

    transport.apply_recorded_session_timing(initial_frame.session_metadata());
    let playback_ui_profile = crate::install_paths::playback_ui_profile_from_paths(
        &product_paths,
        &game_resolver,
        crate::win32::actual_client_input_variant(),
    )?;
    let playback_viewport = playback_ui_profile.physical_viewport();
    let window_width = i32::try_from(playback_viewport.width)
        .map_err(|_| "playback physical width exceeds the Win32 client extent")?;
    let window_height = i32::try_from(playback_viewport.height)
        .map_err(|_| "playback physical height exceeds the Win32 client extent")?;
    if window_width <= 0 || window_height <= 0 {
        return Err("recorded physical client extent is empty".into());
    }
    let initial_viewer_camera = if production_recording {
        let (map_width, map_height) =
            map_viewer::foundation_map_dimensions(&game_resolver, &content_admission)?;
        let camera_state = map_viewer::ViewerMapCameraState::from_admitted_content(
            &game_resolver,
            &content_admission,
            map_width,
            map_height,
            playback_viewport.width,
            playback_viewport.height,
            map_viewer::DEFAULT_VIEWER_CAMERA_SPEED_SETTING,
        )?;
        Some(Camera::from_viewer_state(camera_state)?)
    } else {
        None
    };
    let mut product_map_composition = if production_recording {
        Some(ProductMapCompositionSession::start_for_window_target(
            &game_resolver,
            &content_admission,
            content_manifest,
            terrain_lod_options,
            playback_viewport.width,
            playback_viewport.height,
        )?)
    } else {
        None
    };
    let static_unit_catalogs = game_ui::static_unit_catalogs::admit_static_unit_catalogs(
        provider.static_artifact(game_ui::static_unit_catalogs::TEMPLATES_EQUIPMENT_ARTIFACT),
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
    let mut window = Window::create("replay_app", window_width, window_height)?;
    let (client_w, client_h) = window.client_size()?;
    let mut gfx = Gfx::create(window.hwnd(), client_w, client_h)?;
    let mut last_product_transport_index: Option<usize> = None;
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
                "replay_app: debug map renderer omitted: --bundle/--capture are evidence-only; the product foundation content operation ran before exhaustive family admission"
            );
            None
        }
    };
    let mut overlay = debug_overlay_enabled.then(|| Overlay::create(&gfx, Some(&game_resolver)));
    let mut product_observer_ui = ProductObserverUiRuntime::from_playback_content(
        &gfx,
        &game_resolver,
        &content_admission,
        &playback_ui_profile,
        provider.decision_catalog(),
        Arc::clone(&static_research_catalogs),
        Arc::clone(&static_focus_definitions),
    )?;
    let mut observer_viewer_ui = game_ui::production_renderer::ObserverViewerUiState::default();
    let mut viewer_map_mode = map_viewer::ViewerMapModeSelection::default();
    let mut effective_scripted_map_mode_ids: Vec<String> = Vec::new();
    let mut map_mode_selection_epoch = 0u64;
    observer_viewer_ui.mapmode = game_ui::hud::MapmodeBinding {
        selected_button: 1,
        use_four_button_layout: None,
    };
    let mut occupation_wheel = OccupationWheelAccumulator::default();
    let mut occupation_input_region = None;
    let mut character_wheel_remainder = 0_i64;
    let mut character_input_region = None;
    let mut character_hit_targets = Vec::new();
    let mut character_semantic_state = None;
    let mut occupation_expand_all = true;
    let mut camera = initial_viewer_camera.or_else(|| {
        bundle.as_ref().map(|bundle| {
            Camera::fit(
                bundle.width,
                bundle.height,
                f64::from(gfx.width),
                f64::from(gfx.height),
            )
        })
    });
    let mut uploaded_hour_index: Option<usize> = None;
    let product_observer_occupation = ProductObserverOccupationRuntime::new(
        &game_resolver,
        &content_admission,
        playback_ui_profile.localisation_language(),
    );
    let mut battleplan_selection = BattleplanSelectionState::default();
    let mut selected_xmap13_ids = Vec::<String>::new();
    let mut selected_xmap14_id = None::<String>;
    let mut prepared_xmap13_for_hit_test: Option<map_viewer::PreparedXmap13Frame> = None;
    let mut prepared_xmap14_for_hit_test: Option<map_viewer::PreparedXmap14Frame> = None;
    let mut presented_xmap21_for_hit_test: Option<map_viewer::PreparedMapFurniturePacket> = None;
    let mut day_night_enabled = false;
    let mut pan_keys = PanKeys::default();
    let mut drag: Option<DragState> = None;
    let mut scrub_drag = false;
    let mut wheel_remainder_units = 0_i32;
    let mut cursor: Option<(i32, i32)> = None;
    let mut closing = false;
    let mut frame_timing = PresentedFrameTiming::new(Instant::now());
    let mut map_animation_time = 0.0f32;

    loop {
        let mut cycle_battleplan_owner = false;
        let mut cycle_battleplan_order = false;
        let mut clear_battleplan_selection = false;
        let mut designer_input = DesignerControlInput::default();
        let (events, alive) = window.pump();
        if !alive {
            break;
        }
        product_observer_ui.begin_observer_frame_transaction(&observer_viewer_ui)?;
        let mut event_checkpoint = Some(ReplayWindowEventCheckpoint {
            transport: transport.presentation_checkpoint(),
            pov: pov.clone(),
            pov_picker: pov_picker.clone(),
            viewer_map_mode: viewer_map_mode.clone(),
            map_mode_selection_epoch,
            camera: camera.clone(),
            occupation_wheel: occupation_wheel.clone(),
            character_wheel_remainder,
            occupation_expand_all,
            battleplan_selection: battleplan_selection.clone(),
            selected_xmap13_ids: selected_xmap13_ids.clone(),
            selected_xmap14_id: selected_xmap14_id.clone(),
            day_night_enabled,
            pan_keys: pan_keys.clone(),
            drag: drag.clone(),
            scrub_drag,
            wheel_remainder_units,
            cursor,
            frame_timing,
            map_animation_time,
            occupation_input_region,
            character_input_region,
            character_hit_targets: character_hit_targets.clone(),
            character_semantic_state: character_semantic_state.clone(),
            last_product_transport_index,
        });
        let mut observer_ui_executed = false;
        let mut observer_ui_outcome = None;
        macro_rules! restore_undisplayed_event_state {
            () => {{
                let checkpoint = event_checkpoint
                    .take()
                    .ok_or("window event transaction has no rollback checkpoint")?;
                transport.restore_presentation_checkpoint(checkpoint.transport);
                pov = checkpoint.pov;
                pov_picker = checkpoint.pov_picker;
                viewer_map_mode = checkpoint.viewer_map_mode;
                map_mode_selection_epoch = checkpoint.map_mode_selection_epoch;
                camera = checkpoint.camera;
                occupation_wheel = checkpoint.occupation_wheel;
                character_wheel_remainder = checkpoint.character_wheel_remainder;
                occupation_expand_all = checkpoint.occupation_expand_all;
                battleplan_selection = checkpoint.battleplan_selection;
                selected_xmap13_ids = checkpoint.selected_xmap13_ids;
                selected_xmap14_id = checkpoint.selected_xmap14_id;
                day_night_enabled = checkpoint.day_night_enabled;
                pan_keys = checkpoint.pan_keys;
                drag = checkpoint.drag;
                scrub_drag = checkpoint.scrub_drag;
                wheel_remainder_units = checkpoint.wheel_remainder_units;
                cursor = checkpoint.cursor;
                frame_timing = checkpoint.frame_timing;
                map_animation_time = checkpoint.map_animation_time;
                occupation_input_region = checkpoint.occupation_input_region;
                character_input_region = checkpoint.character_input_region;
                character_hit_targets = checkpoint.character_hit_targets;
                character_semantic_state = checkpoint.character_semantic_state;
                last_product_transport_index = checkpoint.last_product_transport_index;
                uploaded_hour_index = None;
            }};
        }
        let frame_result = (|| -> Result<()> {
        let now = Instant::now();
        let (viewer_clock_micros, dt) = frame_timing.advance_candidate(now)?;
        map_animation_time += dt as f32;

        let mut screen_w = f64::from(gfx.width);
        let mut screen_h = f64::from(gfx.height);
        for event in events {
            match event {
                WindowEvent::Resized { width, height } => {
                    cancel_app_pointer_capture_for_resize(&mut drag, &mut scrub_drag);
                    occupation_input_region = None;
                    character_input_region = None;
                    character_hit_targets.clear();
                    character_semantic_state = None;
                    product_observer_ui
                        .invalidate_for_resize_durably(&mut observer_viewer_ui)?;
                    gfx.resize(width, height)?;
                    refresh_cached_input_viewport(
                        &mut screen_w,
                        &mut screen_h,
                        gfx.width,
                        gfx.height,
                    );
                    if let Some(composition) = product_map_composition.as_mut() {
                        composition.resize_window_target(gfx.width, gfx.height)?;
                    }
                    uploaded_hour_index = None;
                    prepared_xmap13_for_hit_test = None;
                    prepared_xmap14_for_hit_test = None;
                    presented_xmap21_for_hit_test = None;
                    if let Some(camera) = &mut camera {
                        camera.resize(gfx.width, gfx.height)?;
                    }
                    if let Some(checkpoint) = event_checkpoint.as_mut() {
                        cancel_app_pointer_capture_for_resize(
                            &mut checkpoint.drag,
                            &mut checkpoint.scrub_drag,
                        );
                        checkpoint.camera = camera.clone();
                        checkpoint.occupation_input_region = occupation_input_region;
                        checkpoint.character_input_region = character_input_region;
                        checkpoint.character_hit_targets = character_hit_targets.clone();
                        checkpoint.character_semantic_state = character_semantic_state.clone();
                    }
                }
                WindowEvent::KeyDown { vk, shift } => match vk {
                    VK_ESCAPE => {
                        if product_observer_ui
                            .dismiss_xui21_focused(&mut observer_viewer_ui)
                            .is_ok()
                        {
                            } else if product_observer_ui
                                .close_xui08_topmost(&mut observer_viewer_ui)
                            {
                        } else if observer_viewer_ui.abilities.open {
                            product_observer_ui.close_xui23(&mut observer_viewer_ui);
                        } else if observer_viewer_ui.focus.open {
                            product_observer_ui.close_xui10(&mut observer_viewer_ui);
                        } else if observer_viewer_ui.research.panel_open {
                            product_observer_ui.close_xui09(&mut observer_viewer_ui);
                        } else if observer_viewer_ui.decision_panel_open {
                            product_observer_ui.close_xui19(&mut observer_viewer_ui);
                        } else if observer_viewer_ui.power_balance.panel_open {
                            observer_viewer_ui.power_balance.close();
                        } else if observer_viewer_ui.politics.panel_open {
                            observer_viewer_ui.politics.close();
                        } else if observer_viewer_ui.intelligence.panel_open {
                            observer_viewer_ui.intelligence.close();
                        } else if observer_viewer_ui.combat.panel.is_some() {
                            product_observer_ui.close_xui20(&mut observer_viewer_ui);
                        } else if observer_viewer_ui.character_selection.panel
                            != game_ui::character_ui::CharacterPanel::Closed
                        {
                            observer_viewer_ui.character_selection =
                                game_ui::character_ui::CharacterViewerSelection::default();
                        } else if observer_viewer_ui.occupation.panel
                            != game_ui::occupation_compositor::OccupationPanel::Closed
                        {
                            observer_viewer_ui.occupation.panel =
                                game_ui::occupation_compositor::OccupationPanel::Closed;
                        } else if observer_viewer_ui.trade_panel_open {
                            observer_viewer_ui.trade_panel_open = false;
                            observer_viewer_ui.trade =
                                game_ui::trade_layout::CountryTradeViewerState::default();
                        } else if product_observer_ui.close_xui26_frontend()? {
                        } else {
                            closing = true;
                            window.request_close();
                        }
                    }
                    VK_RETURN | VK_SPACE => {
                        let authored_input_active =
                            product_observer_ui.authored_input_window_active();
                        if let Some(action) = product_observer_ui.authored_activate_focused()? {
                            let disposition = product_observer_ui
                                .dispatch_authenticated_viewer_action(&action)?;
                            if apply_authenticated_viewer_disposition(
                                &disposition,
                                &mut transport,
                                &mut observer_viewer_ui,
                                &mut viewer_map_mode,
                                &mut map_mode_selection_epoch,
                            )? {
                                uploaded_hour_index = None;
                            }
                            eprintln!(
                                "replay_app: authenticated keyboard viewer action {:?} -> {:?}",
                                action.action(),
                                disposition
                            );
                        } else if !authored_input_active && vk == VK_SPACE {
                            transport.toggle_play();
                        }
                    }
                    VK_LEFT | VK_OEM_4 => {
                        transport.step_backward();
                        product_observer_ui.reset_xui23_for_seek(&mut observer_viewer_ui);
                        reset_xui17_for_seek(
                            &mut observer_viewer_ui,
                            &mut character_input_region,
                            &mut character_hit_targets,
                            &mut character_semantic_state,
                        );
                    }
                    VK_RIGHT | VK_OEM_6 => {
                        transport.step_forward();
                        product_observer_ui.reset_xui23_for_seek(&mut observer_viewer_ui);
                        reset_xui17_for_seek(
                            &mut observer_viewer_ui,
                            &mut character_input_region,
                            &mut character_hit_targets,
                            &mut character_semantic_state,
                        );
                    }
                    VK_OEM_PLUS | VK_ADD => transport.speed_up(),
                    VK_OEM_MINUS | VK_SUBTRACT => transport.speed_down(),
                    VK_OEM_COMMA => {
                        let date_ticks = transport.date_ticks_with(|hour| {
                            provider.date_at(hour).map_err(|error| error.to_string())
                        })?;
                            let here = transport.current_index();
                            if let Some(tick) =
                                date_ticks.iter().rev().find(|tick| tick.index < here)
                            {
                                transport.seek_to_index(tick.index);
                            product_observer_ui.reset_xui23_for_seek(&mut observer_viewer_ui);
                            reset_xui17_for_seek(
                                &mut observer_viewer_ui,
                                &mut character_input_region,
                                &mut character_hit_targets,
                                &mut character_semantic_state,
                            );
                        }
                    }
                    VK_OEM_PERIOD => {
                        let date_ticks = transport.date_ticks_with(|hour| {
                            provider.date_at(hour).map_err(|error| error.to_string())
                        })?;
                        let here = transport.current_index();
                        if let Some(tick) = date_ticks.iter().find(|tick| tick.index > here) {
                            transport.seek_to_index(tick.index);
                            product_observer_ui.reset_xui23_for_seek(&mut observer_viewer_ui);
                            reset_xui17_for_seek(
                                &mut observer_viewer_ui,
                                &mut character_input_region,
                                &mut character_hit_targets,
                                &mut character_semantic_state,
                            );
                        }
                    }
                    VK_O => pov.toggle(),
                    VK_TAB => {
                        let direction = if shift {
                            game_ui::window_manager_law::ReplayTraversalDirection::Backward
                        } else {
                            game_ui::window_manager_law::ReplayTraversalDirection::Forward
                        };
                        if product_observer_ui
                            .cycle_xui21_focus(&mut observer_viewer_ui, shift)
                            .is_ok()
                        {
                        } else if !product_observer_ui.authored_keyboard_traverse(direction)? {
                            if let PovPicker::Available(choices) = &pov_picker {
                                let next = pov
                                    .selected_ui_country()
                                    .and_then(|selected| {
                                        choices.iter().position(|choice| choice.tag == selected)
                                    })
                                    .map_or(0, |index| (index + 1) % choices.len());
                                if let Some(choice) = choices.get(next) {
                                    pov.select_ui_country(&choice.tag);
                                    if apply_selected_country_identity_with_runtime_reset(
                                        &mut product_observer_ui,
                                        &mut observer_viewer_ui,
                                        Some(choice.tag.clone()),
                                    ) {
                                        reset_xui17_for_seek(
                                            &mut observer_viewer_ui,
                                            &mut character_input_region,
                                            &mut character_hit_targets,
                                            &mut character_semantic_state,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    digit if (VK_1..=VK_9).contains(&digit) => {
                        if let PovPicker::Available(choices) = &pov_picker {
                            let slot = (digit - VK_1) as usize;
                            if let Some(choice) = choices.get(slot) {
                                pov.select_ui_country(&choice.tag);
                                if apply_selected_country_identity_with_runtime_reset(
                                    &mut product_observer_ui,
                                    &mut observer_viewer_ui,
                                    Some(choice.tag.clone()),
                                ) {
                                    reset_xui17_for_seek(
                                        &mut observer_viewer_ui,
                                        &mut character_input_region,
                                        &mut character_hit_targets,
                                        &mut character_semantic_state,
                                    );
                                }
                            }
                        }
                    }
                    function_key if (VK_F1..=VK_F11).contains(&function_key) => {
                        let button = u8::try_from(function_key - VK_F1 + 1)?;
                            let mode = map_viewer::HardcodedMapMode::from_visible_button(button)
                                .ok_or(
                                    "visible map-mode button has no exact engine registry entry",
                                )?;
                            let selected = map_viewer::ViewerMapModeSelection::Hardcoded(mode);
                        if selected != viewer_map_mode {
                            map_mode_selection_epoch = map_mode_selection_epoch
                                .checked_add(1)
                                .ok_or("viewer map-mode selection epoch overflow")?;
                            viewer_map_mode = selected;
                        }
                        observer_viewer_ui.mapmode = game_ui::hud::MapmodeBinding {
                            selected_button: button,
                            use_four_button_layout: None,
                        };
                        uploaded_hour_index = None;
                    }
                    VK_PRIOR | VK_NEXT if shift && observer_viewer_ui.decision_panel_open => {
                        if let Err(error) = product_observer_ui
                            .xui19_page_scroll(&mut observer_viewer_ui, vk == VK_NEXT)
                        {
                            eprintln!(
                                "replay_app: XUI19 Presented page route unavailable: {error}"
                            );
                        }
                    }
                    VK_PRIOR | VK_NEXT => {
                        let selected = map_viewer::cycle_viewer_map_mode(
                            &viewer_map_mode,
                            &effective_scripted_map_mode_ids,
                            vk == VK_NEXT,
                        )?;
                        if selected != viewer_map_mode {
                            map_mode_selection_epoch = map_mode_selection_epoch
                                .checked_add(1)
                                .ok_or("viewer map-mode selection epoch overflow")?;
                            viewer_map_mode = selected;
                        }
                            observer_viewer_ui.mapmode = game_ui::hud::MapmodeBinding {
                                selected_button: viewer_map_mode
                                    .visible_button()
                                    .unwrap_or_default(),
                                use_four_button_layout: None,
                        };
                        uploaded_hour_index = None;
                    }
                    VK_R => {
                        if let Some(camera) = &mut camera {
                            camera.reset_fit(screen_w, screen_h)?;
                        }
                    }
                    VK_D if shift && pov.selected_ui_country().is_some() => {
                        match product_observer_ui.toggle_xui19(&mut observer_viewer_ui) {
                            Ok(()) if observer_viewer_ui.decision_panel_open => {
                                product_observer_ui.close_xui08(&mut observer_viewer_ui);
                                product_observer_ui.close_xui20(&mut observer_viewer_ui);
                                product_observer_ui.close_xui10(&mut observer_viewer_ui);
                                product_observer_ui.close_xui09(&mut observer_viewer_ui);
                                product_observer_ui.close_xui23(&mut observer_viewer_ui);
                                observer_viewer_ui.power_balance.close();
                                observer_viewer_ui.politics.close();
                                observer_viewer_ui.intelligence.close();
                                observer_viewer_ui.character_selection =
                                    game_ui::character_ui::CharacterViewerSelection::default();
                                observer_viewer_ui.designer = None;
                                observer_viewer_ui.production_lines.panel_open = false;
                                observer_viewer_ui.trade_panel_open = false;
                                observer_viewer_ui.occupation.panel =
                                    game_ui::occupation_compositor::OccupationPanel::Closed;
                            }
                            Ok(()) => {}
                            Err(error) => eprintln!(
                                "replay_app: XUI19 Presented passive route unavailable: {error}"
                            ),
                        }
                    }
                    VK_I if shift && pov.selected_ui_country().is_some() => {
                        let result = if observer_viewer_ui.intelligence.panel_open {
                            product_observer_ui.cycle_xui14_surface(&mut observer_viewer_ui)
                        } else {
                            product_observer_ui.toggle_xui14(&mut observer_viewer_ui)
                        };
                        match result {
                            Ok(()) if observer_viewer_ui.intelligence.panel_open => {
                                product_observer_ui.close_xui08(&mut observer_viewer_ui);
                                product_observer_ui.close_xui19(&mut observer_viewer_ui);
                                product_observer_ui.close_xui20(&mut observer_viewer_ui);
                                product_observer_ui.close_xui10(&mut observer_viewer_ui);
                                product_observer_ui.close_xui09(&mut observer_viewer_ui);
                                product_observer_ui.close_xui23(&mut observer_viewer_ui);
                                observer_viewer_ui.politics.close();
                                observer_viewer_ui.character_selection =
                                    game_ui::character_ui::CharacterViewerSelection::default();
                                observer_viewer_ui.designer = None;
                                observer_viewer_ui.production_lines.panel_open = false;
                                observer_viewer_ui.trade_panel_open = false;
                                observer_viewer_ui.occupation.panel =
                                    game_ui::occupation_compositor::OccupationPanel::Closed;
                            }
                            Ok(()) => {}
                            Err(error) => eprintln!(
                                "replay_app: XUI14 Presented passive route unavailable: {error}"
                            ),
                        }
                    }
                    VK_I if pov.selected_ui_country().is_some() => {
                        match product_observer_ui.toggle_xui11(&mut observer_viewer_ui) {
                            Ok(()) if observer_viewer_ui.politics.panel_open => {
                                product_observer_ui.close_xui19(&mut observer_viewer_ui);
                                product_observer_ui.close_xui20(&mut observer_viewer_ui);
                                product_observer_ui.close_xui10(&mut observer_viewer_ui);
                                product_observer_ui.close_xui09(&mut observer_viewer_ui);
                                product_observer_ui.close_xui23(&mut observer_viewer_ui);
                                observer_viewer_ui.intelligence.close();
                                observer_viewer_ui.character_selection =
                                    game_ui::character_ui::CharacterViewerSelection::default();
                                observer_viewer_ui.designer = None;
                                observer_viewer_ui.production_lines.panel_open = false;
                                observer_viewer_ui.trade_panel_open = false;
                                observer_viewer_ui.occupation.panel =
                                    game_ui::occupation_compositor::OccupationPanel::Closed;
                            }
                            Ok(()) => {}
                            Err(error) => eprintln!(
                                "replay_app: XUI11 Presented passive route unavailable: {error}"
                            ),
                        }
                    }
                    VK_X if pov.selected_ui_country().is_some() => {
                        match product_observer_ui.cycle_xui23_context(&mut observer_viewer_ui) {
                            Ok(()) if observer_viewer_ui.abilities.open => {
                                product_observer_ui.close_xui19(&mut observer_viewer_ui);
                                product_observer_ui.close_xui09(&mut observer_viewer_ui);
                                observer_viewer_ui.character_selection =
                                    game_ui::character_ui::CharacterViewerSelection::default();
                                observer_viewer_ui.designer = None;
                                observer_viewer_ui.production_lines.panel_open = false;
                                observer_viewer_ui.trade_panel_open = false;
                                observer_viewer_ui.occupation.panel =
                                    game_ui::occupation_compositor::OccupationPanel::Closed;
                            }
                            Ok(()) => {}
                            Err(error) => eprintln!(
                                "replay_app: XUI23 Presented context route unavailable: {error}"
                            ),
                        }
                    }
                    VK_Y if pov.selected_ui_country().is_some() => {
                        use game_ui::character_ui::{CharacterPanel, CharacterUiAction};
                        let action = match &observer_viewer_ui.character_selection.panel {
                                CharacterPanel::Closed => Some(CharacterUiAction::OpenArmyRoster),
                                CharacterPanel::ArmyRoster => {
                                    Some(CharacterUiAction::OpenNavyRoster)
                                }
                                CharacterPanel::NavyRoster => Some(CharacterUiAction::OpenAces),
                            CharacterPanel::Aces | CharacterPanel::TraitTree { .. } => {
                                Some(CharacterUiAction::Close)
                            }
                        };
                        if let Some(action) = action {
                                if let Some(model) = character_semantic_state.as_ref() {
                                    if let Err(error) =
                                        game_ui::character_ui::apply_character_action(
                                            &mut observer_viewer_ui.character_selection,
                                    model,
                                            action,
                                        )
                                    {
                                        eprintln!(
                                        "replay_app: XUI17 Presented lifecycle transition rejected: {error}"
                                    );
                                }
                            } else {
                                eprintln!(
                                    "replay_app: XUI17 Presented lifecycle transition rejected: no admitted Presented model"
                                );
                            }
                        }
                        observer_viewer_ui.character_selection.scroll_row = 0;
                            character_wheel_remainder = 0;
                            if observer_viewer_ui.character_selection.panel
                                != CharacterPanel::Closed
                            {
                                product_observer_ui.close_xui19(&mut observer_viewer_ui);
                            product_observer_ui.close_xui09(&mut observer_viewer_ui);
                            observer_viewer_ui.designer = None;
                            observer_viewer_ui.production_lines.panel_open = false;
                            observer_viewer_ui.trade_panel_open = false;
                            observer_viewer_ui.occupation.panel =
                                game_ui::occupation_compositor::OccupationPanel::Closed;
                        }
                    }
                    VK_H if matches!(
                        &observer_viewer_ui.character_selection.panel,
                        game_ui::character_ui::CharacterPanel::ArmyRoster
                            | game_ui::character_ui::CharacterPanel::NavyRoster
                            | game_ui::character_ui::CharacterPanel::Aces
                    ) =>
                    {
                        observer_viewer_ui.character_selection.scroll_row = observer_viewer_ui
                            .character_selection
                            .scroll_row
                            .saturating_sub(1);
                    }
                    VK_J if matches!(
                        &observer_viewer_ui.character_selection.panel,
                        game_ui::character_ui::CharacterPanel::ArmyRoster
                            | game_ui::character_ui::CharacterPanel::NavyRoster
                            | game_ui::character_ui::CharacterPanel::Aces
                    ) =>
                    {
                        observer_viewer_ui.character_selection.scroll_row = observer_viewer_ui
                            .character_selection
                            .scroll_row
                            .saturating_add(1);
                    }
                    VK_E if shift && pov.selected_ui_country().is_some() => {
                        match product_observer_ui.toggle_xui09(&mut observer_viewer_ui) {
                            Ok(()) if observer_viewer_ui.research.panel_open => {
                                product_observer_ui.close_xui19(&mut observer_viewer_ui);
                                product_observer_ui.close_xui20(&mut observer_viewer_ui);
                                product_observer_ui.close_xui10(&mut observer_viewer_ui);
                                product_observer_ui.close_xui23(&mut observer_viewer_ui);
                                observer_viewer_ui.politics.close();
                                observer_viewer_ui.intelligence.close();
                                observer_viewer_ui.character_selection =
                                    game_ui::character_ui::CharacterViewerSelection::default();
                                observer_viewer_ui.designer = None;
                                observer_viewer_ui.production_lines.panel_open = false;
                                observer_viewer_ui.trade_panel_open = false;
                                observer_viewer_ui.occupation.panel =
                                    game_ui::occupation_compositor::OccupationPanel::Closed;
                            }
                            Ok(()) => {}
                            Err(error) => eprintln!(
                                "replay_app: XUI09 Presented passive route unavailable: {error}"
                            ),
                        }
                    }
                    VK_F if shift && observer_viewer_ui.research.panel_open => {
                        if let Err(error) =
                            product_observer_ui.cycle_xui09_folder(&mut observer_viewer_ui)
                        {
                            eprintln!(
                                "replay_app: XUI09 Presented authored-folder roster unavailable: {error}"
                            );
                        }
                    }
                    VK_E if pov.selected_ui_country().is_some()
                        && observer_viewer_ui.character_selection.panel
                            == game_ui::character_ui::CharacterPanel::Closed
                        && observer_viewer_ui.occupation.panel
                            == game_ui::occupation_compositor::OccupationPanel::Closed =>
                    {
                        product_observer_ui.close_xui19(&mut observer_viewer_ui);
                        product_observer_ui.close_xui09(&mut observer_viewer_ui);
                        designer_input.toggle_open = true;
                    }
                    VK_F if observer_viewer_ui.designer.is_some() => {
                        designer_input.cycle_family = true;
                    }
                    VK_G if observer_viewer_ui.designer.is_some() => {
                        designer_input.cycle_selection = true;
                    }
                    VK_H if observer_viewer_ui.designer.is_some() => {
                        designer_input.scroll_delta = -1;
                    }
                    VK_J if observer_viewer_ui.designer.is_some() => {
                        designer_input.scroll_delta = 1;
                    }
                    VK_Q if observer_viewer_ui.designer.is_some() && shift => {
                        designer_input.toggle_country_filter = true;
                    }
                    VK_Q if observer_viewer_ui.designer.is_some() => {
                        designer_input.country_scroll_delta = -1;
                    }
                    VK_L if observer_viewer_ui.designer.is_some() => {
                        designer_input.country_scroll_delta = 1;
                    }
                    VK_K if observer_viewer_ui.designer.is_some() && shift => {
                        designer_input.toggle_foreign = true;
                    }
                    VK_K if observer_viewer_ui.designer.is_some() => {
                        designer_input.toggle_our = true;
                    }
                    VK_V if observer_viewer_ui.designer.is_some() => {
                        designer_input.toggle_obsolete = true;
                    }
                        VK_T if pov.selected_ui_country().is_some() => {
                            observer_viewer_ui.trade_panel_open =
                                !observer_viewer_ui.trade_panel_open;
                            if observer_viewer_ui.trade_panel_open {
                            product_observer_ui.close_xui19(&mut observer_viewer_ui);
                            product_observer_ui.close_xui09(&mut observer_viewer_ui);
                            observer_viewer_ui.character_selection =
                                game_ui::character_ui::CharacterViewerSelection::default();
                            observer_viewer_ui.designer = None;
                            observer_viewer_ui.production_lines.panel_open = false;
                            observer_viewer_ui.occupation.panel =
                                game_ui::occupation_compositor::OccupationPanel::Closed;
                        }
                    }
                    VK_P if shift => {
                        match product_observer_ui.toggle_xui08(&mut observer_viewer_ui) {
                                Ok(())
                                    if product_observer_ui
                                        .xui08_will_be_open(&observer_viewer_ui) =>
                                {
                                product_observer_ui.close_xui19(&mut observer_viewer_ui);
                                product_observer_ui.close_xui20(&mut observer_viewer_ui);
                                product_observer_ui.close_xui10(&mut observer_viewer_ui);
                                product_observer_ui.close_xui09(&mut observer_viewer_ui);
                                product_observer_ui.close_xui23(&mut observer_viewer_ui);
                                observer_viewer_ui.politics.close();
                                observer_viewer_ui.intelligence.close();
                                observer_viewer_ui.character_selection =
                                    game_ui::character_ui::CharacterViewerSelection::default();
                                observer_viewer_ui.designer = None;
                                observer_viewer_ui.production_lines.panel_open = false;
                                observer_viewer_ui.trade_panel_open = false;
                                observer_viewer_ui.occupation.panel =
                                    game_ui::occupation_compositor::OccupationPanel::Closed;
                            }
                            Ok(()) => {}
                            Err(error) => eprintln!(
                                "replay_app: XUI08 Presented passive route unavailable: {error}"
                            ),
                        }
                    }
                    VK_P if pov.selected_ui_country().is_some() => {
                        observer_viewer_ui.production_lines.panel_open =
                            !observer_viewer_ui.production_lines.panel_open;
                        if observer_viewer_ui.production_lines.panel_open {
                            product_observer_ui.close_xui19(&mut observer_viewer_ui);
                            product_observer_ui.close_xui09(&mut observer_viewer_ui);
                            observer_viewer_ui.character_selection =
                                game_ui::character_ui::CharacterViewerSelection::default();
                            observer_viewer_ui.designer = None;
                            observer_viewer_ui.trade_panel_open = false;
                            observer_viewer_ui.occupation.panel =
                                game_ui::occupation_compositor::OccupationPanel::Closed;
                        }
                    }
                    VK_U => {
                        product_observer_ui.close_xui19(&mut observer_viewer_ui);
                        product_observer_ui.close_xui09(&mut observer_viewer_ui);
                        let frame = scrub
                            .shown()
                            .frame
                            .clone()
                            .ok_or("XUI26 product route has no current WorldFrame")?;
                        let route = product_observer_ui.cycle_frontend_panel(
                            &frame,
                            gfx.width as usize,
                            gfx.height as usize,
                            viewer_clock_micros,
                        )?;
                        eprintln!("replay_app: XUI26 frontend route -> {route:?}");
                    }
                    VK_G if pov.selected_ui_country().is_some() => {
                            use game_ui::occupation_compositor::OccupationPanel;
                            observer_viewer_ui.occupation.panel = match observer_viewer_ui
                                .occupation
                                .panel
                            {
                                OccupationPanel::Closed => OccupationPanel::Territories,
                                OccupationPanel::Territories => OccupationPanel::GarrisonActivities,
                                OccupationPanel::GarrisonActivities => {
                                    OccupationPanel::GarrisonEquipment
                                }
                                OccupationPanel::GarrisonEquipment => OccupationPanel::Closed,
                            };
                        if observer_viewer_ui.occupation.panel != OccupationPanel::Closed {
                            product_observer_ui.close_xui19(&mut observer_viewer_ui);
                            product_observer_ui.close_xui09(&mut observer_viewer_ui);
                            observer_viewer_ui.character_selection =
                                game_ui::character_ui::CharacterViewerSelection::default();
                            observer_viewer_ui.designer = None;
                            observer_viewer_ui.production_lines.panel_open = false;
                            observer_viewer_ui.trade_panel_open = false;
                        }
                    }
                    VK_V if occupation_controls_active(
                        &observer_viewer_ui,
                        pov.selected_ui_country(),
                    ) =>
                    {
                        use game_ui::occupation_compositor::OccupationTerritoryMetric;
                        observer_viewer_ui.occupation.territory_metric =
                            match observer_viewer_ui.occupation.territory_metric {
                                OccupationTerritoryMetric::Resistance => {
                                    OccupationTerritoryMetric::Compliance
                                }
                                OccupationTerritoryMetric::Compliance => {
                                    OccupationTerritoryMetric::Resistance
                                }
                            };
                    }
                    VK_F if occupation_controls_active(
                        &observer_viewer_ui,
                        pov.selected_ui_country(),
                    ) =>
                    {
                        observer_viewer_ui.occupation.show_non_resisting =
                            !observer_viewer_ui.occupation.show_non_resisting;
                    }
                    VK_S if occupation_controls_active(
                        &observer_viewer_ui,
                        pov.selected_ui_country(),
                    ) =>
                    {
                        observer_viewer_ui.occupation.territory_sort_mode =
                            (observer_viewer_ui.occupation.territory_sort_mode + 1) % 5;
                    }
                    VK_K if occupation_controls_active(
                        &observer_viewer_ui,
                        pov.selected_ui_country(),
                    ) =>
                    {
                        observer_viewer_ui.occupation.garrison_months =
                            if observer_viewer_ui.occupation.garrison_months == 12 {
                                1
                            } else {
                                observer_viewer_ui.occupation.garrison_months + 1
                            };
                    }
                    VK_E if occupation_controls_active(
                        &observer_viewer_ui,
                        pov.selected_ui_country(),
                    ) =>
                    {
                        occupation_expand_all = !occupation_expand_all
                    }
                    VK_N => {
                        day_night_enabled = !day_night_enabled;
                    }
                    VK_C if shift => {
                        match product_observer_ui.cycle_xui20_panel(&mut observer_viewer_ui) {
                            Ok(()) => {
                                product_observer_ui.close_xui19(&mut observer_viewer_ui);
                                product_observer_ui.close_xui10(&mut observer_viewer_ui);
                                product_observer_ui.close_xui09(&mut observer_viewer_ui);
                                product_observer_ui.close_xui23(&mut observer_viewer_ui);
                                observer_viewer_ui.politics.close();
                                observer_viewer_ui.intelligence.close();
                                observer_viewer_ui.character_selection =
                                    game_ui::character_ui::CharacterViewerSelection::default();
                                observer_viewer_ui.designer = None;
                                observer_viewer_ui.production_lines.panel_open = false;
                                observer_viewer_ui.trade_panel_open = false;
                                observer_viewer_ui.occupation.panel =
                                    game_ui::occupation_compositor::OccupationPanel::Closed;
                            }
                            Err(error) => eprintln!(
                                "replay_app: XUI20 Presented passive-global route unavailable: {error}"
                            ),
                        }
                    }
                    VK_B => cycle_battleplan_owner = true,
                    VK_M => cycle_battleplan_order = true,
                    VK_C => clear_battleplan_selection = true,
                    VK_A => pan_keys.west = true,
                    VK_D => pan_keys.east = true,
                    VK_W => pan_keys.north = true,
                    VK_S => pan_keys.south = true,
                    _ => {}
                },
                WindowEvent::KeyUp { vk } => match vk {
                    VK_A => pan_keys.west = false,
                    VK_D => pan_keys.east = false,
                    VK_W => pan_keys.north = false,
                    VK_S => pan_keys.south = false,
                    _ => {}
                },
                WindowEvent::MouseMove { x, y } => {
                    cursor = Some((x, y));
                        product_observer_ui.xui21_pointer_move(x, y);
                        let xui21_owns_pointer = product_observer_ui
                            .xui21_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        if !xui21_owns_pointer {
                            product_observer_ui.xui19_pointer_move(x, y);
                            product_observer_ui.xui08_pointer_move(
                                &mut observer_viewer_ui,
                                x,
                                y,
                            )?;
                            product_observer_ui.xui11_pointer_move(x, y);
                        product_observer_ui.xui18_pointer_move(x, y);
                        product_observer_ui.xui14_pointer_move(x, y);
                        product_observer_ui.xui17_pointer_move(x, y);
                        product_observer_ui.xui20_pointer_move(x, y);
                    }
                        let xui09_dragged = !xui21_owns_pointer
                            && product_observer_ui.xui09_pointer_move(
                                &mut observer_viewer_ui,
                                x,
                                y,
                            )?;
                        let xui09_owns_pointer = observer_viewer_ui.research.panel_open
                        && (xui09_dragged
                            || product_observer_ui.xui09_blocks_lower_pointer_input(
                                &observer_viewer_ui,
                                x,
                                y,
                            ));
                        let xui10_dragged = !xui21_owns_pointer
                            && product_observer_ui.xui10_pointer_move(
                                &mut observer_viewer_ui,
                                x,
                                y,
                            )?;
                        let xui10_owns_pointer = xui10_dragged
                            || product_observer_ui.xui10_blocks_lower_pointer_input(x, y);
                        let xui23_owns_pointer =
                            product_observer_ui.xui23_blocks_lower_pointer_input(x, y);
                        let xui17_owns_pointer =
                            character_owns_pointer(character_input_region, x, y);
                        let xui16_owns_pointer =
                            occupation_owns_pointer(occupation_input_region, x, y);
                        let xui11_owns_pointer =
                            product_observer_ui.xui11_blocks_lower_pointer_input(x, y);
                        let xui18_owns_pointer = product_observer_ui
                            .xui18_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui19_owns_pointer = product_observer_ui
                            .xui19_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui08_owns_pointer = product_observer_ui
                            .xui08_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui14_owns_pointer = product_observer_ui
                            .xui14_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui20_owns_pointer =
                        product_observer_ui.xui20_blocks_lower_pointer_input(x, y);
                    if xui21_owns_pointer
                        || xui09_owns_pointer
                        || xui10_owns_pointer
                        || xui23_owns_pointer
                        || xui17_owns_pointer
                        || xui16_owns_pointer
                        || xui08_owns_pointer
                        || xui11_owns_pointer
                        || xui18_owns_pointer
                        || xui19_owns_pointer
                        || xui14_owns_pointer
                        || xui20_owns_pointer
                    {
                        product_observer_ui.cancel_authored_pointer_capture()?;
                        product_observer_ui.xui15_pointer_release(&mut observer_viewer_ui);
                        scrub_drag = false;
                        drag = None;
                    } else {
                        let xui15_owns_pointer = product_observer_ui
                                .xui15_blocks_lower_pointer_input(x, y)?
                                || product_observer_ui
                                    .xui15_has_pointer_capture(&observer_viewer_ui);
                            if xui15_owns_pointer {
                                product_observer_ui.cancel_authored_pointer_capture()?;
                                product_observer_ui
                                    .xui15_pointer_move(&mut observer_viewer_ui, y)?;
                                scrub_drag = false;
                            drag = None;
                            } else {
                                product_observer_ui.authored_pointer_move(
                                    x,
                                    y,
                                    viewer_clock_micros,
                                )?;
                                apply_production_lines_scroll_offset(
                                &mut observer_viewer_ui,
                                product_observer_ui.production_lines_scroll_offset()?,
                            );
                            if debug_overlay_enabled && scrub_drag {
                                if let Some(fraction) = overlay::scrub_drag_fraction(
                                    x as f32,
                                    screen_w as f32,
                                    screen_h as f32,
                                ) {
                                    transport.seek_to_fraction(f64::from(fraction));
                                    product_observer_ui
                                        .reset_xui23_for_seek(&mut observer_viewer_ui);
                                    reset_xui17_for_seek(
                                        &mut observer_viewer_ui,
                                        &mut character_input_region,
                                        &mut character_hit_targets,
                                        &mut character_semantic_state,
                                    );
                                }
                            }
                            if let Some(state) = &drag {
                                if let Some(camera) = &mut camera {
                                    let dx = f64::from(x - state.start_cursor.0);
                                    let dy = f64::from(y - state.start_cursor.1);
                                    camera.drag_from(&state.start_camera, dx, dy, screen_h)?;
                                }
                            }
                        }
                    }
                }
                WindowEvent::MouseDown { button, x, y } => {
                    let xui21_blocks_lower_input = product_observer_ui
                        .xui21_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                    let xui09_blocks_lower_input = product_observer_ui
                        .xui09_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                    let xui10_blocks_lower_input =
                        product_observer_ui.xui10_blocks_lower_pointer_input(x, y);
                    let xui23_blocks_lower_input =
                        product_observer_ui.xui23_blocks_lower_pointer_input(x, y);
                    let xui17_blocks_lower_input =
                        character_owns_pointer(character_input_region, x, y);
                    let xui16_blocks_lower_input =
                        occupation_owns_pointer(occupation_input_region, x, y);
                    let xui11_blocks_lower_input =
                        product_observer_ui.xui11_blocks_lower_pointer_input(x, y);
                    let xui18_blocks_lower_input = product_observer_ui
                        .xui18_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                    let xui19_blocks_lower_input = product_observer_ui
                        .xui19_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                    let xui08_blocks_lower_input = product_observer_ui
                        .xui08_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                    let xui14_blocks_lower_input = product_observer_ui
                        .xui14_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                    let xui20_blocks_lower_input =
                        product_observer_ui.xui20_blocks_lower_pointer_input(x, y);
                    let xui15_blocks_lower_input =
                        product_observer_ui.xui15_blocks_lower_pointer_input(x, y)?;
                    if !xui21_blocks_lower_input
                        && !xui09_blocks_lower_input
                        && !xui10_blocks_lower_input
                        && !xui23_blocks_lower_input
                        && !xui17_blocks_lower_input
                        && !xui16_blocks_lower_input
                        && !xui08_blocks_lower_input
                        && !xui11_blocks_lower_input
                        && !xui18_blocks_lower_input
                        && !xui19_blocks_lower_input
                        && !xui14_blocks_lower_input
                        && !xui20_blocks_lower_input
                        && !xui15_blocks_lower_input
                    {
                        product_observer_ui.authored_pointer_move(x, y, viewer_clock_micros)?;
                    }
                    let authored_blocks_lower_input =
                        product_observer_ui.authored_pointer_blocks_lower_input(x, y)?;
                    match button {
                        MouseButton::Left => {
                            if xui21_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    let disposition = product_observer_ui.xui21_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                )?;
                                if let crate::ui_runtime::ObserverViewerActionDisposition::RefusedEventOptionActivation {
                                    event_sequence,
                                    definition_ordinal,
                                } = disposition
                                {
                                    eprintln!(
                                        "replay_app: XUI21 read-only viewer refused gameplay option event_sequence={event_sequence} ordinal={definition_ordinal}"
                                    );
                                }
                                scrub_drag = false;
                                drag = None;
                            } else if xui19_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    let disposition = product_observer_ui.xui19_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                )?;
                                match disposition {
                                    crate::ui_runtime::ObserverViewerActionDisposition::RefusedDecisionActivation {
                                        source_record_id,
                                        decision_id,
                                    } => eprintln!(
                                        "replay_app: XUI19 read-only viewer refused gameplay decision {decision_id} record={source_record_id}"
                                    ),
                                    crate::ui_runtime::ObserverViewerActionDisposition::RefusedDecisionCategoryProviderActivation {
                                        category_id,
                                        provider_id,
                                        stable_namespace,
                                    } => eprintln!(
                                        "replay_app: XUI19 read-only viewer refused category provider {provider_id} category={category_id} owner={stable_namespace}"
                                    ),
                                    _ => {}
                                }
                                scrub_drag = false;
                                drag = None;
                            } else if xui09_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    product_observer_ui.xui09_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                    true,
                                )?;
                                scrub_drag = false;
                                drag = None;
                            } else if xui10_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    product_observer_ui.xui10_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                    true,
                                )?;
                                scrub_drag = false;
                                drag = None;
                            } else if xui23_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    if let Some(disposition) =
                                    product_observer_ui.xui23_pointer_press(x, y)
                                {
                                    apply_authenticated_viewer_disposition(
                                        &disposition,
                                        &mut transport,
                                        &mut observer_viewer_ui,
                                        &mut viewer_map_mode,
                                        &mut map_mode_selection_epoch,
                                    )?;
                                }
                                scrub_drag = false;
                                drag = None;
                            } else if xui17_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    if let (Some(action), Some(model)) = (
                                    game_ui::character_ui::hit_test_character_panel(
                                        &character_hit_targets,
                                        x,
                                        y,
                                    ),
                                    character_semantic_state.as_ref(),
                                ) {
                                    if let Err(error) =
                                        game_ui::character_ui::apply_character_action(
                                            &mut observer_viewer_ui.character_selection,
                                            model,
                                            action,
                                        )
                                    {
                                        eprintln!(
                                            "replay_app: XUI17 Presented action rejected: {error}"
                                        );
                                    }
                                }
                                scrub_drag = false;
                                drag = None;
                            } else if xui16_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    scrub_drag = false;
                                drag = None;
                            } else if xui08_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    product_observer_ui.xui08_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                )?;
                                scrub_drag = false;
                                drag = None;
                            } else if xui18_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    product_observer_ui.xui18_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                )?;
                                scrub_drag = false;
                                drag = None;
                            } else if xui11_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    product_observer_ui.xui11_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                )?;
                                scrub_drag = false;
                                drag = None;
                            } else if xui14_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    product_observer_ui.xui14_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                )?;
                                scrub_drag = false;
                                drag = None;
                            } else if xui20_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    product_observer_ui.xui20_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                );
                                scrub_drag = false;
                                drag = None;
                            } else if xui15_blocks_lower_input {
                                product_observer_ui.cancel_authored_pointer_capture()?;
                                product_observer_ui.xui15_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                )?;
                                scrub_drag = false;
                                drag = None;
                            } else if authored_blocks_lower_input {
                                product_observer_ui.authored_pointer_press(
                                    x,
                                    y,
                                    viewer_clock_micros,
                                )?;
                                scrub_drag = false;
                            } else {
                                let mut consumed_by_unit_selection = false;
                                if debug_overlay_enabled {
                                    if let Some(fraction) = overlay::scrub_fraction_at(
                                        x as f32,
                                        y as f32,
                                        screen_w as f32,
                                        screen_h as f32,
                                    ) {
                                        transport.seek_to_fraction(f64::from(fraction));
                                        product_observer_ui
                                            .reset_xui23_for_seek(&mut observer_viewer_ui);
                                        reset_xui17_for_seek(
                                            &mut observer_viewer_ui,
                                            &mut character_input_region,
                                            &mut character_hit_targets,
                                            &mut character_semantic_state,
                                        );
                                        scrub_drag = true;
                                        consumed_by_unit_selection = true;
                                    }
                                }
                                if production_recording && !consumed_by_unit_selection {
                                    selected_xmap13_ids.clear();
                                    selected_xmap14_id = None;
                                    let furniture_hits = presented_xmap21_for_hit_test
                                        .as_ref()
                                        .map(|packet| {
                                            map_viewer::hit_test_prepared_map_furniture(
                                                packet,
                                                [x as f32, y as f32],
                                            )
                                        })
                                        .transpose()?
                                        .unwrap_or_default();
                                    let combat_hit = prepared_xmap14_for_hit_test
                                        .as_ref()
                                        .and_then(|combat| combat.hit_test(x as f32, y as f32));
                                    let unit_hit = prepared_xmap13_for_hit_test
                                        .as_ref()
                                        .and_then(|units| units.hit_test(x as f32, y as f32));
                                    match select_topmost_product_map_icon_hit(
                                        furniture_hits,
                                        combat_hit,
                                        unit_hit,
                                    ) {
                                        Some(ProductMapIconPointerHit::Furniture(target)) => {
                                            selected_xmap13_ids.push(target.canonical_id());
                                        }
                                        Some(ProductMapIconPointerHit::Combat(identity)) => {
                                            selected_xmap14_id = Some(identity.canonical_id());
                                        }
                                        Some(ProductMapIconPointerHit::Unit(identity)) => {
                                            selected_xmap13_ids.push(identity.canonical_id());
                                        }
                                        None => {}
                                    }
                                }
                            }
                        }
                        MouseButton::Middle | MouseButton::Right => {
                            if xui21_blocks_lower_input || xui19_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    drag = None;
                                } else if button == MouseButton::Middle && xui09_blocks_lower_input
                                {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    product_observer_ui.xui09_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                    false,
                                )?;
                                    drag = None;
                                } else if button == MouseButton::Middle && xui10_blocks_lower_input
                                {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    product_observer_ui.xui10_pointer_press(
                                    &mut observer_viewer_ui,
                                    x,
                                    y,
                                    false,
                                )?;
                                drag = None;
                            } else if xui08_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    drag = None;
                            } else if xui18_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    drag = None;
                            } else if xui11_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    drag = None;
                            } else if xui14_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    drag = None;
                            } else if xui20_blocks_lower_input {
                                    product_observer_ui.cancel_authored_pointer_capture()?;
                                    product_observer_ui
                                        .xui15_pointer_release(&mut observer_viewer_ui);
                                    drag = None;
                            } else if product_map_camera_drag_allowed(
                                button,
                                xui20_blocks_lower_input,
                                xui09_blocks_lower_input
                                    || xui21_blocks_lower_input
                                    || xui10_blocks_lower_input
                                    || xui23_blocks_lower_input
                                    || xui17_blocks_lower_input
                                    || xui16_blocks_lower_input
                                    || xui08_blocks_lower_input
                                    || xui18_blocks_lower_input
                                    || xui11_blocks_lower_input
                                    || xui14_blocks_lower_input
                                    || xui15_blocks_lower_input
                                    || authored_blocks_lower_input,
                            ) {
                                if let Some(camera) = &camera {
                                    drag = Some(DragState {
                                        start_cursor: (x, y),
                                        start_camera: camera.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
                WindowEvent::MouseUp { button, x, y } => {
                    let xui08_had_capture = button == MouseButton::Left
                        && product_observer_ui.xui08_pointer_release(&mut observer_viewer_ui);
                    let xui09_had_capture =
                        matches!(button, MouseButton::Left | MouseButton::Middle)
                            && product_observer_ui.xui09_pointer_release();
                    let xui10_had_capture =
                        matches!(button, MouseButton::Left | MouseButton::Middle)
                            && product_observer_ui.xui10_pointer_release();
                    if button == MouseButton::Left {
                        let xui21_owns_pointer = product_observer_ui
                            .xui21_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui09_owns_pointer = product_observer_ui
                            .xui09_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui10_owns_pointer =
                            product_observer_ui.xui10_blocks_lower_pointer_input(x, y);
                        let xui23_owns_pointer =
                            product_observer_ui.xui23_blocks_lower_pointer_input(x, y);
                        let xui17_owns_pointer =
                            character_owns_pointer(character_input_region, x, y);
                        let xui16_owns_pointer =
                            occupation_owns_pointer(occupation_input_region, x, y);
                        let xui08_owns_pointer = product_observer_ui
                            .xui08_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui11_owns_pointer =
                            product_observer_ui.xui11_blocks_lower_pointer_input(x, y);
                        let xui18_owns_pointer = product_observer_ui
                            .xui18_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui14_owns_pointer = product_observer_ui
                            .xui14_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui15_owns_pointer = product_observer_ui
                                .xui15_blocks_lower_pointer_input(x, y)?
                                || product_observer_ui
                                    .xui15_has_pointer_capture(&observer_viewer_ui);
                            let release = if xui21_owns_pointer
                            || xui08_had_capture
                            || xui09_had_capture
                            || xui09_owns_pointer
                            || xui10_had_capture
                            || xui10_owns_pointer
                            || xui23_owns_pointer
                            || xui17_owns_pointer
                            || xui16_owns_pointer
                            || xui08_owns_pointer
                            || xui11_owns_pointer
                            || xui18_owns_pointer
                            || xui14_owns_pointer
                        {
                            product_observer_ui.xui15_pointer_release(&mut observer_viewer_ui);
                            product_observer_ui.cancel_authored_pointer_capture()?;
                            None
                        } else if xui15_owns_pointer {
                            product_observer_ui.xui15_pointer_release(&mut observer_viewer_ui);
                            product_observer_ui.cancel_authored_pointer_capture()?;
                            None
                        } else {
                            product_observer_ui.authored_pointer_release(
                                x,
                                y,
                                viewer_clock_micros,
                            )?
                        };
                        if let Some(release) = release {
                            if let Some(action) = release.activated_action() {
                                let disposition = product_observer_ui
                                    .dispatch_authenticated_viewer_action(action)?;
                                if apply_authenticated_viewer_disposition(
                                    &disposition,
                                    &mut transport,
                                    &mut observer_viewer_ui,
                                    &mut viewer_map_mode,
                                    &mut map_mode_selection_epoch,
                                )? {
                                    uploaded_hour_index = None;
                                }
                                eprintln!(
                                    "replay_app: authenticated viewer action {:?} -> {:?}",
                                    action.action(),
                                    disposition
                                );
                            }
                        }
                        scrub_drag = false;
                    } else {
                        drag = None;
                    }
                }
                WindowEvent::Wheel { delta, x, y } => {
                    cursor = Some((x, y));
                    product_observer_ui.xui21_pointer_move(x, y);
                    product_observer_ui.xui19_pointer_move(x, y);
                    let xui21_blocks_lower_input = product_observer_ui
                        .xui21_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                    let xui19_blocks_lower_input = product_observer_ui
                        .xui19_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                    let emitted_notches = if xui21_blocks_lower_input {
                        0
                    } else {
                        accumulate_engine_wheel_notches(&mut wheel_remainder_units, delta)?
                    };
                    let xui19_consumed = !xui21_blocks_lower_input
                        && emitted_notches != 0
                        && product_observer_ui.xui19_wheel_notches_at(
                            &mut observer_viewer_ui,
                            x,
                            y,
                            emitted_notches,
                        )?;
                    let xui09_blocks_lower_input = product_observer_ui
                        .xui09_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                    let xui10_blocks_lower_input =
                        product_observer_ui.xui10_blocks_lower_pointer_input(x, y);
                    let xui23_blocks_lower_input =
                        product_observer_ui.xui23_blocks_lower_pointer_input(x, y);
                    let xui17_blocks_lower_input =
                        character_owns_pointer(character_input_region, x, y);
                    let xui17_scrolled = !xui21_blocks_lower_input
                        && !xui19_blocks_lower_input
                        && route_character_wheel(
                            &mut observer_viewer_ui.character_selection,
                            &mut character_wheel_remainder,
                            character_input_region,
                            delta,
                            x,
                            y,
                        );
                    let xui16_blocks_lower_input =
                        occupation_owns_pointer(occupation_input_region, x, y);
                    let xui16_scrolled = !xui21_blocks_lower_input
                        && !xui19_blocks_lower_input
                        && route_occupation_wheel(
                            &mut observer_viewer_ui.occupation,
                            &mut occupation_wheel,
                            occupation_input_region,
                            delta,
                            x,
                            y,
                        );
                    if !xui21_blocks_lower_input
                        && !xui19_consumed
                        && !xui19_blocks_lower_input
                        && !xui09_blocks_lower_input
                        && !xui10_blocks_lower_input
                        && !xui23_blocks_lower_input
                        && !xui17_scrolled
                        && !xui17_blocks_lower_input
                        && !xui16_scrolled
                        && !xui16_blocks_lower_input
                    {
                        let xui08_blocks_lower_input = product_observer_ui
                            .xui08_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui08_consumed = !xui19_consumed
                            && !xui19_blocks_lower_input
                            && emitted_notches != 0
                            && product_observer_ui.xui08_wheel_notches_at(
                                &mut observer_viewer_ui,
                                x,
                                y,
                                emitted_notches,
                            )?;
                        let xui20_blocks_lower_input =
                            product_observer_ui.xui20_blocks_lower_pointer_input(x, y);
                        let xui20_consumed = !xui08_consumed
                            && !xui08_blocks_lower_input
                            && !xui19_consumed
                            && !xui19_blocks_lower_input
                            && emitted_notches != 0
                            && product_observer_ui.xui20_wheel_notches_at(
                                &mut observer_viewer_ui,
                                x,
                                y,
                                emitted_notches,
                            )?;
                        let xui11_blocks_lower_input =
                            product_observer_ui.xui11_blocks_lower_pointer_input(x, y);
                        let xui18_blocks_lower_input = product_observer_ui
                            .xui18_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui11_consumed = !xui20_consumed
                            && !xui20_blocks_lower_input
                            && !xui19_consumed
                            && !xui19_blocks_lower_input
                            && !xui18_blocks_lower_input
                            && emitted_notches != 0
                            && product_observer_ui.xui11_wheel_notches_at(
                                &mut observer_viewer_ui,
                                x,
                                y,
                                emitted_notches,
                            )?;
                        let xui14_blocks_lower_input = product_observer_ui
                            .xui14_blocks_lower_pointer_input(&observer_viewer_ui, x, y);
                        let xui14_consumed = !xui20_consumed
                            && !xui20_blocks_lower_input
                            && !xui19_consumed
                            && !xui19_blocks_lower_input
                            && !xui11_consumed
                            && !xui11_blocks_lower_input
                            && !xui18_blocks_lower_input
                            && emitted_notches != 0
                            && product_observer_ui.xui14_wheel_notches_at(
                                &mut observer_viewer_ui,
                                x,
                                y,
                                emitted_notches,
                            );
                        let xui15_blocks_lower_input =
                            product_observer_ui.xui15_blocks_lower_pointer_input(x, y)?;
                        let xui15_consumed = !xui20_consumed
                            && !xui20_blocks_lower_input
                            && !xui19_consumed
                            && !xui19_blocks_lower_input
                            && !xui11_consumed
                            && !xui11_blocks_lower_input
                            && !xui18_blocks_lower_input
                            && !xui14_consumed
                            && !xui14_blocks_lower_input
                            && emitted_notches != 0
                            && product_observer_ui.xui15_wheel_notches_at(
                                &mut observer_viewer_ui,
                                x,
                                y,
                                emitted_notches,
                                )?;
                            let authored_consumed = if emitted_notches == 0
                                || xui08_consumed
                                || xui08_blocks_lower_input
                                || xui19_consumed
                                || xui19_blocks_lower_input
                                || xui11_consumed
                                || xui20_consumed
                                || xui20_blocks_lower_input
                                || xui11_blocks_lower_input
                                || xui18_blocks_lower_input
                                || xui14_consumed
                                || xui14_blocks_lower_input
                                || xui15_consumed
                                || xui15_blocks_lower_input
                            {
                                false
                            } else {
                                let outcome = product_observer_ui
                                    .scroll_authored_wheel_notches_at(x, y, emitted_notches)?;
                                if let Some(outcome) = outcome.as_ref() {
                                    let production_offset = product_observer_ui
                                        .production_lines_scroll_offset_for_outcome(outcome)?;
                                    apply_production_lines_scroll_offset(
                                        &mut observer_viewer_ui,
                                        production_offset,
                                    );
                                }
                                outcome.is_some()
                            };
                        let authored_blocks_lower_input =
                            product_observer_ui.authored_pointer_blocks_lower_input(x, y)?;
                        if !xui15_consumed
                            && !xui08_consumed
                            && !xui19_consumed
                            && !xui11_consumed
                            && !xui14_consumed
                            && !xui20_consumed
                            && !authored_consumed
                            && !xui08_blocks_lower_input
                            && !xui19_blocks_lower_input
                            && !xui11_blocks_lower_input
                            && !xui18_blocks_lower_input
                            && !xui14_blocks_lower_input
                            && !xui20_blocks_lower_input
                            && !xui15_blocks_lower_input
                            && !authored_blocks_lower_input
                        {
                            if let Some(camera) = &mut camera {
                                if camera.viewer_state().is_some() && emitted_notches != 0 {
                                    camera.wheel_zoom(
                                        f64::from(x) + 0.5,
                                        f64::from(y) + 0.5,
                                        emitted_notches,
                                        dt,
                                    )?;
                                }
                            }
                        }
                    }
                    drag = None;
                }
                WindowEvent::CaptureLost => {
                    product_observer_ui.xui08_capture_lost(&mut observer_viewer_ui);
                    product_observer_ui.xui09_capture_lost();
                    product_observer_ui.xui10_capture_lost();
                    product_observer_ui.xui15_capture_lost(&mut observer_viewer_ui);
                    product_observer_ui.cancel_authored_pointer_capture_durably()?;
                    drag = None;
                    scrub_drag = false;
                    if let Some(checkpoint) = event_checkpoint.as_mut() {
                        checkpoint.drag = None;
                        checkpoint.scrub_drag = false;
                    }
                }
            }
        }
        if closing {
            product_observer_ui.reject_unexecuted_observer_frame(&mut observer_viewer_ui)?;
            restore_undisplayed_event_state!();
            return Ok(());
        }

        if let Some(camera) = &mut camera {
            if camera.viewer_state().is_none() {
                let pan_x = (f64::from(pan_keys.east as u8) - f64::from(pan_keys.west as u8))
                    * LEGACY_DEBUG_KEY_PAN_SCREEN_PX_PER_SEC
                    * dt;
                let pan_y = (f64::from(pan_keys.south as u8) - f64::from(pan_keys.north as u8))
                    * LEGACY_DEBUG_KEY_PAN_SCREEN_PX_PER_SEC
                    * dt;
                if pan_x != 0.0 || pan_y != 0.0 {
                    camera.pan_screen(pan_x, pan_y, f64::from(gfx.height))?;
                }
            }
            camera.advance(dt)?;
        }

        transport.advance(dt);

        let requested_hour = transport.current_hour();
        let (displayed, scrub_status) = scrub.show(requested_hour);
        let hour = displayed.hour;
        let world_frame = displayed.frame.clone();
        let hour_index = hours.partition_point(|recorded| *recorded < hour);
        let mut current_product_render = None;
        if let Some(frame) = world_frame.as_deref() {
            pov_picker = povs_from_frame(frame);
            match &pov_picker {
                PovPicker::Available(choices) => {
                    if pov.selected_ui_country().is_some_and(|selected| {
                        !choices.iter().any(|choice| choice.tag == selected)
                    }) {
                        pov.clear_country_selection();
                    }
                }
                PovPicker::Unavailable(_) => pov.clear_country_selection(),
            }
            } else {
                pov_picker = PovPicker::Unavailable(
                    "displayed frame has no typed d18 country roster".into(),
                );
                pov.clear_country_selection();
        }
        if production_recording {
            let frame = world_frame
                .as_deref()
                .ok_or_else(|| map_viewer::refuse_missing_xmap00_authority("world_frame"))?;
            let product_camera = camera
                .as_ref()
                .ok_or("XMAP03 product route lost its viewer-owned camera")?;
            let viewer = current_camera_viewer_state(
                product_camera,
                pov.selected_ui_country(),
                &viewer_map_mode,
                map_mode_selection_epoch,
                &selected_xmap13_ids,
                map_animation_time,
                dt as f32,
            )?;
            let xmap15_selection = battleplan_selection.xmap15_selection(frame);
            current_product_render = Some((viewer, xmap15_selection));
        }
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
                let research_bindings =
                    static_research_catalogs
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
                    research_bindings.technology_sharing_definition_unavailable_ordinals,
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
                if let Some(country) = pov.selected_ui_country() {
                    let authored_technologies = static_research_catalogs
                        .admitted()
                        .idea_technology()
                        .technology_ids()
                        .map(|id| id.to_string())
                        .collect::<Vec<_>>();
                    let coloring = game_ui::observer_technology::observer_technology_coloring(
                        frame,
                        country,
                        &authored_technologies,
                    );
                    if let Some(gap) = &coloring.gap {
                        let identity = format!("OBSERVER_TECH_COLORING\0{country}\0{gap:?}");
                        if reported_production_data_needs.insert(identity) {
                            eprintln!(
                                "replay_app: completed-technology coloring for observer country {} unavailable: {:?}",
                                country, gap
                            );
                        }
                    }
                }
            }
            let owners = Arc::clone(&displayed.owners);
            let controllers = Arc::clone(&displayed.controllers);
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
            uploaded_hour_index = Some(hour_index);
        }

        let probe = match (cursor, renderer.as_ref(), camera.as_ref(), bundle.as_ref()) {
            (Some((x, y)), Some(renderer), Some(camera), Some(bundle)) => {
                let map_pos = renderer.screen_to_map(
                    camera,
                    f64::from(x) + 0.5,
                    f64::from(y) + 0.5,
                    f64::from(gfx.width),
                    f64::from(gfx.height),
                )?;
                probe_at(bundle, &displayed.owners, &displayed.controllers, camera, map_pos)?
            }
            _ => String::from("map omitted"),
        };
        let playing = if transport.is_playing() {
            " [playing]"
        } else {
            ""
        };
        let observer_render_status = pov.render_status(world_frame.as_deref());
        let observer_render_admitted =
            !matches!(&observer_render_status, PovRenderStatus::Unavailable { .. });
        let observer_note = match &observer_render_status {
            PovRenderStatus::Omniscient => format!(
                "observer ui: {}; fog: world",
                pov.selected_ui_country().unwrap_or("none")
            ),
            PovRenderStatus::Recorded {
                pov: fog_pov,
                province_slot_count,
                spotted_land_unit_count,
                spotted_taskforce_count,
                ..
            } => format!(
                "observer ui: {}; fog: {fog_pov} (d94 provinces {province_slot_count}; d95 land {spotted_land_unit_count:?}, taskforces {spotted_taskforce_count:?})",
                pov.selected_ui_country().unwrap_or("none")
            ),
            PovRenderStatus::Unavailable {
                pov: fog_pov,
                diagnostic,
            } => {
                format!(
                    "observer ui: {}; fog: {fog_pov} unavailable ({diagnostic})",
                    pov.selected_ui_country().unwrap_or("none")
                )
            }
        };
        let speed = transport.speed_hours_per_sec();
        let hour_label = match &scrub_status {
            crate::scrub::ScrubStatus::Settled => displayed.label.clone(),
            crate::scrub::ScrubStatus::Converging { requested } => {
                format!("{} | seeking to hour {requested}", displayed.label)
            }
            crate::scrub::ScrubStatus::Refused { requested, reason } => {
                format!("{} | hour {requested} unavailable: {reason}", displayed.label)
            }
        };
        if let crate::scrub::ScrubStatus::Refused { requested, reason } = &scrub_status {
            if last_reported_scrub_refusal != Some(*requested) {
                eprintln!("replay_app: hour {requested} cannot be shown: {reason}");
                last_reported_scrub_refusal = Some(*requested);
            }
        } else {
            last_reported_scrub_refusal = None;
        }
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
        let anomaly_note = displayed.anomaly_note.clone();
        let anomaly_suffix = anomaly_note
            .as_deref()
            .map(|note| format!(" | {note}"))
            .unwrap_or_default();
        let combat_selection_suffix = selected_xmap14_id
            .as_deref()
            .map(|identity| format!(" | combat {identity}"))
            .unwrap_or_default();
        if overlay.as_ref().is_some_and(Overlay::has_text) {
            window.set_title(&format!(
                "replay_app - {mode_label}{recorded_clock_suffix}{combat_selection_suffix}{anomaly_suffix}"
            ));
        } else {
            let zoom = camera.as_ref().map(|camera| camera.zoom);
            window.set_title(&format!(
                "replay_app - {mode_label} | {hour_label}{playing} {speed:.0}h/s{recorded_clock_suffix} | {observer_note} | {probe}{}{combat_selection_suffix}{anomaly_suffix}",
                zoom.map(|value| format!(" | zoom {value:.2}")).unwrap_or_default()
            ));
        }

        if gfx.width > 0 && gfx.height > 0 {
                let observer_occupation_outcome = world_frame.as_ref().map(|frame| {
                    product_observer_occupation.refresh(frame, pov.selected_ui_country())
                });
                let day_night_hour =
                day_night_enabled.then(|| playback_day_night_hour_from_timeline_hour(hour));
            let mut production_map_composed = false;
            let mut xmap22_window_receipt = None;
            let mut pending_product_map_interaction = None;
            if production_recording {
                let current_world_frame = world_frame
                    .as_deref()
                    .ok_or("XMAP10 product route has no current WorldFrame")?;
                let (viewer, xmap15_selection) = current_product_render
                    .as_ref()
                    .ok_or("XMAP22 product route lost its viewer/selection join")?;
                    let current_transport_index = hour_index;
                    let transport_discontinuity =
                        last_product_transport_index.is_some_and(|prior| {
                            current_transport_index != prior
                        && current_transport_index != prior.saturating_add(1)
                });
                let composition = product_map_composition
                    .as_mut()
                    .ok_or("XMAP22 window route has no authoritative composition session")?;
                let receipt = composition.execute_on_target(
                    &gfx,
                    &game_resolver,
                    &content_admission,
                    content_manifest,
                    current_world_frame,
                    viewer,
                    xmap15_selection,
                    playback_ui_profile.localisation_language(),
                    dt as f32,
                    transport_discontinuity,
                    day_night_enabled,
                    playback_ui_profile.key().gui_scale(),
                )?;
                last_product_transport_index = Some(current_transport_index);
                let prepared = composition
                    .last_prepared()
                    .ok_or("XMAP22 completed without retaining its prepared frame")?;
                pending_product_map_interaction = Some((
                    prepared.map_modes.effective_scripted_mode_ids.clone(),
                    prepared.units.clone(),
                    prepared.combat.clone(),
                    composition.map_furniture_packet().cloned(),
                ));
                production_map_composed = true;
                xmap22_window_receipt = Some(receipt);
            }
            let mut legacy_presented = production_map_composed;
            if observer_render_admitted && !production_recording {
                if let (Some(renderer), Some(camera)) = (renderer.as_mut(), camera.as_ref()) {
                    renderer.render(&gfx, camera, day_night_hour, None)?;
                    legacy_presented = true;
                }
            }
            if !legacy_presented {
                gfx.clear_target([0.0, 0.0, 0.0, 1.0])?;
            }
            if observer_render_admitted {
                let current_world_frame = world_frame
                    .as_deref()
                    .ok_or("observer UI has no current WorldFrame")?;
                if apply_selected_country_identity_with_runtime_reset(
                    &mut product_observer_ui,
                    &mut observer_viewer_ui,
                    pov.selected_ui_country().map(str::to_string),
                ) {
                    reset_xui17_for_seek(
                        &mut observer_viewer_ui,
                        &mut character_input_region,
                        &mut character_hit_targets,
                        &mut character_semantic_state,
                    );
                }
                apply_designer_controls(
                    &product_observer_ui,
                    current_world_frame,
                    pov.selected_ui_country(),
                    &mut observer_viewer_ui,
                    designer_input,
                );
                observer_viewer_ui.mapmode = authenticated_observer_mapmode_binding(
                    &viewer_map_mode,
                    mapmode_uses_four_button_layout(current_world_frame),
                );
                if observer_viewer_ui.selected_country.is_none() {
                    observer_viewer_ui.occupation.panel =
                        game_ui::occupation_compositor::OccupationPanel::Closed;
                    observer_viewer_ui.occupation.selected_country.clear();
                } else if let Some(selected) = observer_viewer_ui.selected_country.as_deref() {
                    observer_viewer_ui.occupation.selected_country = selected.to_owned();
                }
                observer_viewer_ui.occupation.expanded_country_tags.clear();
                if occupation_expand_all
                    && observer_viewer_ui.occupation.panel
                        == game_ui::occupation_compositor::OccupationPanel::Territories
                {
                    if let (Some(selected), Some(supplement)) = (
                        observer_viewer_ui.selected_country.as_deref(),
                        current_world_frame
                            .occupation_ui_supplement_exact()
                            .supplement
                            .as_ref(),
                    ) {
                        if let Some(controller) = supplement.controllers.get(selected) {
                            observer_viewer_ui
                                .occupation
                                .expanded_country_tags
                                .extend(controller.presentation.country_groups.keys().cloned());
                        }
                    }
                }
                let outcome = match product_observer_ui.execute(
                    &gfx,
                    current_world_frame,
                    &mut observer_viewer_ui,
                    transport.speed_step(),
                    transport.is_playing(),
                    viewer_clock_micros,
                    ProductionLinesUiSupplementalInputs::from_frame(current_world_frame),
                    observer_occupation_outcome.as_ref(),
                ) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        product_observer_ui
                            .rollback_after_execute_error(&mut observer_viewer_ui)
                            .map_err(|rollback_error| {
                                format!(
                                    "observer UI execute failed ({error}); XUI20 rollback also failed: {rollback_error}"
                                )
                            })?;
                        restore_undisplayed_event_state!();
                        return Err(error);
                    }
                };
                observer_ui_executed = true;
                observer_ui_outcome = Some(outcome);
            }
            let line2 = format!(
                "{mode_label}{playing} {speed:.0}h/s | {POLITICAL_COLOR_BLOCKER_BADGE} | {observer_note} | {probe}{anomaly_suffix}"
            );
            if debug_overlay_enabled && !observer_ui_executed {
                if let Some(overlay) = &mut overlay {
                    overlay.draw(&gfx, &hour_label, &line2, hour_index, transport.len())?;
                }
            }
            let present_outcome = match gfx.present() {
                Ok(outcome) => outcome,
                Err(present_error) => {
                    if production_recording {
                        product_map_composition
                            .as_mut()
                            .ok_or("XMAP22 Present failure lost its composition session")?
                            .finish_window_present(crate::d3d::PresentOutcome::Skipped)?;
                    }
                    if observer_ui_executed {
                        product_observer_ui
                            .reject_unpresented_authored_frame(&mut observer_viewer_ui)
                            .map_err(|rollback_error| {
                                format!(
                                    "DXGI Present failed ({present_error}); authenticated frame rollback also failed: {rollback_error}"
                                )
                            })?;
                    } else {
                        product_observer_ui
                            .reject_unexecuted_observer_frame(&mut observer_viewer_ui)?;
                    }
                    restore_undisplayed_event_state!();
                    return Err(present_error);
                }
            };
            if production_recording {
                product_map_composition
                    .as_mut()
                    .ok_or("XMAP22 Present disposition lost its composition session")?
                    .finish_window_present(present_outcome)?;
            }
            let xmap22_window_receipt = xmap22_window_receipt
                .map(|receipt| receipt.finish_window_present(present_outcome))
                .transpose()?;
            if production_recording && xmap22_window_receipt.is_none() {
                    return Err(
                        "XMAP22 production window reached Present without a composition receipt"
                            .into(),
                    );
            }
                if let Some(receipt) = &xmap22_window_receipt {
                    let identity =
                        format!("XMAP22_WINDOW_PRESENT\0{:?}", receipt.presentation_status);
                    if reported_production_data_needs.insert(identity) {
                    eprintln!(
                        "replay_app: XMAP22 shared window composition {:?}: receipt_sha256={} final_pixels_sha256={} family_census_sha256={}",
                        receipt.presentation_status,
                        receipt.deterministic_sha256,
                        receipt.final_pixels_sha256,
                        receipt.mandatory_family_execution_census_sha256,
                    );
                }
            }
            match present_outcome {
                crate::d3d::PresentOutcome::Presented => {
                    event_checkpoint.take();
                    if let Some((scripted_modes, units, combat, furniture)) =
                        pending_product_map_interaction.take()
                    {
                        effective_scripted_map_mode_ids = scripted_modes;
                        prepared_xmap13_for_hit_test = units;
                        if selected_xmap14_id.as_ref().is_some_and(|selected| {
                            combat.as_ref().is_none_or(|prepared| {
                                !prepared
                                    .indicators
                                    .iter()
                                    .any(|row| row.identity.canonical_id() == *selected)
                            })
                        }) {
                            selected_xmap14_id = None;
                        }
                        prepared_xmap14_for_hit_test = combat;
                        presented_xmap21_for_hit_test = furniture;
                    }
                    let production_lines_candidate_offset = observer_ui_outcome
                        .as_ref()
                            .and_then(|outcome| outcome.production_lines_applied_scroll_offset);
                        product_observer_ui.finish_observer_frame_disposition(
                            &mut observer_viewer_ui,
                        ObserverUiFrameDisposition::Presented,
                        observer_ui_executed,
                        production_lines_candidate_offset,
                    )?;
                    if !observer_ui_executed {
                        occupation_input_region = None;
                        character_input_region = None;
                        character_hit_targets.clear();
                        character_semantic_state = None;
                    }
                    if let Some(mut outcome) = observer_ui_outcome {
                        apply_occupation_scroll_feedback(
                            &mut observer_viewer_ui.occupation,
                            outcome.occupation_applied_scroll_row,
                        );
                        occupation_input_region = outcome.occupation_input_region;
                        apply_character_scroll_feedback(
                            &mut observer_viewer_ui.character_selection,
                            outcome.character_applied_scroll_row,
                        );
                        character_input_region = outcome.character_input_region;
                        character_hit_targets = outcome.character_hit_targets;
                            character_semantic_state = outcome.character_semantic_state;
                            outcome.authored_window_snapshot = product_observer_ui
                                .authored_window_snapshot(viewer_clock_micros)?;
                            let identity = format!(
                            "OBSERVER_UI\0{:?}\0{}\0{}\0{:?}\0{}\0{:?}\0{}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{}\0{:?}\0{}",
                            outcome.selected_country,
                            outcome.trade_panel_open,
                            outcome.production_lines_open,
                            outcome.production_lines_country,
                            outcome.production_lines_available,
                            outcome.authored_roots,
                            outcome.authored_panel_draw_count,
                            outcome.authored_window_snapshot,
                            outcome.gaps,
                            outcome.semantic_receipts,
                            outcome.feature_local_gaps,
                            outcome.promoted_extension_packets,
                            observer_occupation_outcome,
                            outcome.characters_open,
                            outcome.characters_country,
                            outcome.characters_available,
                        );
                        if reported_production_data_needs.insert(identity) {
                            eprintln!(
                                "replay_app: state-first observer UI presented for {:?}; authored roots {:?}; authored_panel_draws={}; authored_window_snapshot={:?}; trade_open={}; XUI12_open={}; XUI12_country={:?}; XUI12_available={}; compositor gaps={:?}; semantic receipts={:?}; semantic gaps={:?}; promoted extensions={:?}; XUI16={:?}",
                                outcome.selected_country,
                                outcome.authored_roots,
                                outcome.authored_panel_draw_count,
                                outcome.authored_window_snapshot,
                                outcome.trade_panel_open,
                                outcome.production_lines_open,
                                outcome.production_lines_country,
                                outcome.production_lines_available,
                                outcome.gaps,
                                outcome.semantic_receipts,
                                outcome.feature_local_gaps,
                                outcome.promoted_extension_packets,
                                observer_occupation_outcome
                            );
                        }
                    }
                }
                    crate::d3d::PresentOutcome::Skipped => {
                        product_observer_ui.finish_observer_frame_disposition(
                            &mut observer_viewer_ui,
                        ObserverUiFrameDisposition::Skipped,
                        observer_ui_executed,
                        observer_ui_outcome
                            .as_ref()
                            .and_then(|outcome| outcome.production_lines_applied_scroll_offset),
                    )?;
                    restore_undisplayed_event_state!();
                    std::thread::sleep(std::time::Duration::from_millis(4));
                }
                    crate::d3d::PresentOutcome::Occluded => {
                        product_observer_ui.finish_observer_frame_disposition(
                            &mut observer_viewer_ui,
                        ObserverUiFrameDisposition::Occluded,
                        observer_ui_executed,
                        observer_ui_outcome
                            .as_ref()
                            .and_then(|outcome| outcome.production_lines_applied_scroll_offset),
                    )?;
                    restore_undisplayed_event_state!();
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
            } else {
                product_observer_ui.finish_observer_frame_disposition(
                    &mut observer_viewer_ui,
                ObserverUiFrameDisposition::Skipped,
                false,
                None,
            )?;
            restore_undisplayed_event_state!();
        }
            Ok(())
        })();
        if let Err(error) = frame_result {
            if product_map_composition
                .as_ref()
                .is_some_and(ProductMapCompositionSession::has_pending_window_present)
            {
                product_map_composition
                    .as_mut()
                    .expect("checked pending XMAP10 presentation session")
                    .finish_window_present(crate::d3d::PresentOutcome::Skipped)?;
            }
            if event_checkpoint.is_some() {
                product_observer_ui.finish_observer_frame_disposition(
                    &mut observer_viewer_ui,
                    ObserverUiFrameDisposition::ExecuteError,
                    observer_ui_executed,
                    None,
                )?;
                restore_undisplayed_event_state!();
            }
            return Err(error);
        }
    }
    Ok(())
}

fn probe_at(
    bundle: &MapBundle,
    owners: &BTreeMap<u32, String>,
    controllers: &BTreeMap<u32, String>,
    camera: &Camera,
    map_pos: Option<(f64, f64)>,
) -> Result<String> {
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
    use std::path::PathBuf;

    #[test]
    fn rejected_frame_timing_is_reapplied_to_the_next_presented_candidate() {
        let origin = Instant::now();
        let mut timing = PresentedFrameTiming::new(origin);
        let checkpoint = timing;

        let (_, rejected_delta) = timing
            .advance_candidate(origin + std::time::Duration::from_millis(20))
            .unwrap();
        assert!((rejected_delta - 0.020).abs() < f64::EPSILON);

        timing = checkpoint;
        let (_, presented_delta) = timing
            .advance_candidate(origin + std::time::Duration::from_millis(35))
            .unwrap();
        assert!((presented_delta - 0.035).abs() < f64::EPSILON);

        let (_, next_presented_delta) = timing
            .advance_candidate(origin + std::time::Duration::from_millis(45))
            .unwrap();
        assert!((next_presented_delta - 0.010).abs() < f64::EPSILON);
    }

    #[test]
    fn xmap22_composition_receipt_is_content_bound_and_deny_unknown() {
        let hash = || "a".repeat(64);
        let mandatory_family_execution_census = map_viewer::MapVisualFamily::ALL
            .into_iter()
            .map(|family| Xmap22FamilyExecutionRow {
                family,
                owner_packet: family.owner_packet().into(),
                foundation_disposition: map_viewer::FamilyDisposition::Present {
                    payload_schema: "test.present.v1".into(),
                    producer_registration_sha256: hash(),
                    payload_sha256: hash(),
                    ordered_operations: Vec::new(),
                    ordered_used_content: Vec::new(),
                },
                execution: Xmap22FamilyExecutionDisposition::Executed {
                    execution_sha256: hash(),
                },
            })
            .collect::<Vec<_>>();
        let mandatory_family_execution_census_sha256 = snapshot::sha256::Sha256::hex(
            &serde_json::to_vec(&mandatory_family_execution_census).unwrap(),
        );
        let mut receipt = ProductMapCompositionReceipt {
            schema: "hoi4.replay-map-composition.xmap22.v4".into(),
            hour_index: 42,
            viewport_width: 320,
            viewport_height: 180,
            content_manifest_sha256: hash(),
            product_modset_sha256: hash(),
            effective_vfs_manifest_sha256: hash(),
            replay_frame_identity_sha256: hash(),
            viewer_state_sha256: hash(),
            xmap15_viewer_selection_sha256: hash(),
            assigned_stage_topology_sha256: hash(),
            assigned_stage_schedule_sha256: hash(),
            foundation_envelope_sha256: hash(),
            foundation_content_operation_sha256: hash(),
            xmap04_terrain_execution_sha256: hash(),
            xmap05_resource_execution_sha256: hash(),
            xmap06_thin_borders_execution_sha256: hash(),
            xmap07_water_execution_sha256: hash(),
            xmap07_snow_mud_authority_sha256: hash(),
            xmap08_rivers_straits_execution_sha256: hash(),
            xmap09_lighting_execution_sha256: hash(),
            xmap10_postfx_receipt_sha256: hash(),
            xmap20_scene_receipt_sha256: hash(),
            xmap11_tree_execution_sha256: hash(),
            xmap12_execution_sha256: hash(),
            xmap13_unit_execution_sha256: hash(),
            xmap13_counter_execution_sha256: hash(),
            xmap14_execution_sha256: hash(),
            xmap15_execution_sha256: hash(),
            xmap16_rail_execution_sha256: hash(),
            prepared_map_modes_sha256: hash(),
            prepared_map_mode_composition: "political".into(),
            xmap16_supply: Xmap16SupplyCompositionDisposition::MapModeInactive {
                prepared_map_modes_sha256: hash(),
            },
            xmap17_execution_sha256: hash(),
            xmap19_weather_execution_sha256: hash(),
            xmap21_execution_sha256: hash(),
            xmap16_hub: Xmap16HubCompositionDisposition::NoObserverCountry {
                viewer_state_sha256: hash(),
            },
            mandatory_family_count: 20,
            mandatory_family_execution_census,
            mandatory_family_execution_census_sha256,
            foundation_complete: true,
            execution_complete: true,
            final_pixels_measured: true,
            presented_to_product_target: true,
            presentation_status: Xmap22PresentationStatus::OffscreenReadbackMeasured,
            final_target_sha256: hash(),
            final_pixels_sha256: hash(),
            deterministic_sha256: String::new(),
        };
        let expected_witnesses = xmap22_expected_family_execution_witnesses(&receipt).unwrap();
        for row in &mut receipt.mandatory_family_execution_census {
            row.execution = Xmap22FamilyExecutionDisposition::Executed {
                execution_sha256: expected_witnesses[&row.family].clone(),
            };
        }
        receipt.mandatory_family_execution_census_sha256 = snapshot::sha256::Sha256::hex(
            &serde_json::to_vec(&receipt.mandatory_family_execution_census).unwrap(),
        );
        receipt.foundation_envelope_sha256 = xmap22_reconstructed_foundation_envelope(&receipt)
            .unwrap()
            .deterministic_sha256()
            .unwrap();
        let receipt = receipt.seal().expect("seal XMAP22 receipt");
        receipt.validate().expect("validate XMAP22 receipt");

        let mut tampered = receipt.clone();
        tampered.final_pixels_sha256 = "b".repeat(64);
        assert!(tampered.validate().is_err());

        let mut contradictory_foundation_digest = receipt.clone();
        contradictory_foundation_digest.foundation_envelope_sha256 = "b".repeat(64);
        assert!(contradictory_foundation_digest.seal().is_err());

        let mut contradictory_foundation_operation = receipt.clone();
        contradictory_foundation_operation.foundation_content_operation_sha256 = "b".repeat(64);
        assert!(contradictory_foundation_operation.seal().is_err());

        let mut contradictory_foundation_row = receipt.clone();
        let map_viewer::FamilyDisposition::Present { payload_sha256, .. } =
            &mut contradictory_foundation_row.mandatory_family_execution_census[0]
                .foundation_disposition
        else {
            unreachable!()
        };
        *payload_sha256 = "b".repeat(64);
        contradictory_foundation_row.mandatory_family_execution_census_sha256 =
            snapshot::sha256::Sha256::hex(
                &serde_json::to_vec(
                    &contradictory_foundation_row.mandatory_family_execution_census,
                )
                .unwrap(),
            );
        assert!(contradictory_foundation_row.seal().is_err());

        // AN EXCLUDED AND NAMED LAYER MUST STILL SEAL A RECEIPT.
        //
        // Every other case in this fixture is Present, so before this the
        // unavailable path had no coverage at all -- which is how the receipt
        // validator came to carry `unreachable!` for a state its own census
        // builder emits, and how the top-level XMAP14 field came to be
        // unbound (Codex review of batch 3, rounds 1 and 2).
        //
        // kills: restoring `unreachable!` (this panics); dropping the
        // Unavailable arm from the witness check (the valid case refuses);
        // dropping the field binding (the tampered-field case is accepted);
        // loosening the field-for-field gap comparison (the mismatched-gap
        // case is accepted).
        {
            let gap_witness = "c".repeat(64);
            let gap = || map_viewer::FoundationGap {
                gap_id: "XMAP14.combat_indicator_content_unavailable".into(),
                first_unavailable_hour: 42,
                last_unavailable_hour: 42,
                witness_sha256: "c".repeat(64),
                reason: "XMAP14 progress bar GFX_land_unit_combat_progressbar is not authored \
                         horizontal"
                    .into(),
            };
            // Reseal exactly as the producer does: the census digest and the
            // reconstructed foundation-envelope digest are both derived from
            // the rows, so a test that changed a row without re-deriving them
            // would be testing the wrong refusal.
            let reseal = |mut candidate: ProductMapCompositionReceipt| {
                candidate.mandatory_family_execution_census_sha256 =
                    snapshot::sha256::Sha256::hex(
                        &serde_json::to_vec(&candidate.mandatory_family_execution_census).unwrap(),
                    );
                candidate.foundation_envelope_sha256 =
                    xmap22_reconstructed_foundation_envelope(&candidate)
                        .unwrap()
                        .deterministic_sha256()
                        .unwrap();
                candidate.seal()
            };
            let unavailable = |receipt: &ProductMapCompositionReceipt| {
                let mut candidate = receipt.clone();
                let row = candidate
                    .mandatory_family_execution_census
                    .iter_mut()
                    .find(|row| row.family == map_viewer::MapVisualFamily::CombatMarkers)
                    .expect("the census names every family");
                row.foundation_disposition =
                    map_viewer::FamilyDisposition::Unavailable { gap: gap() };
                row.execution = Xmap22FamilyExecutionDisposition::Unavailable {
                    gap_id: gap().gap_id,
                    reason: gap().reason,
                    witness_sha256: gap().witness_sha256,
                };
                candidate.xmap14_execution_sha256 = gap_witness.clone();
                candidate
            };

            reseal(unavailable(&receipt))
                .expect("a named, excluded combat layer still seals a receipt");

            // The flat field a consumer reads must BE the gap's witness.
            let mut tampered = unavailable(&receipt);
            tampered.xmap14_execution_sha256 = "d".repeat(64);
            assert!(
                reseal(tampered).is_err(),
                "an unavailable family's execution field must be the gap witness"
            );

            // And the two halves of the census row must name the same gap.
            let mut mismatched = unavailable(&receipt);
            let row = mismatched
                .mandatory_family_execution_census
                .iter_mut()
                .find(|row| row.family == map_viewer::MapVisualFamily::CombatMarkers)
                .unwrap();
            row.execution = Xmap22FamilyExecutionDisposition::Unavailable {
                gap_id: "XMAP14.exact_combat_indicator_carrier_unavailable".into(),
                reason: gap().reason,
                witness_sha256: gap().witness_sha256,
            };
            assert!(
                reseal(mismatched).is_err(),
                "execution and foundation must name the SAME gap, or the receipt \
                 attributes the missing layer to the wrong cause"
            );

            // A family whose unavailable path has NOT been built has no bound
            // execution field, and that is a REFUSAL rather than a pass --
            // otherwise the flat digest of every other family goes unbound the
            // moment it is marked unavailable (Codex review of batch 3,
            // round 3).
            //
            // kills: returning to the permissive `is_some_and` shape, which
            // accepted an unavailable family with no bound field at all.
            let mut unbound = unavailable(&receipt);
            let row = unbound
                .mandatory_family_execution_census
                .iter_mut()
                .find(|row| row.family == map_viewer::MapVisualFamily::ThinBorders)
                .expect("the census names every family");
            row.foundation_disposition =
                map_viewer::FamilyDisposition::Unavailable { gap: gap() };
            row.execution = Xmap22FamilyExecutionDisposition::Unavailable {
                gap_id: gap().gap_id,
                reason: gap().reason,
                witness_sha256: gap().witness_sha256,
            };
            assert!(
                reseal(unbound).is_err(),
                "an unavailable family with no bound execution field must refuse"
            );
        }

        // The compositor's own choice, without a GPU: an executed layer uses its
        // receipt digest, an excluded and NAMED layer uses the gap's witness,
        // and nothing else is ever a witness.
        //
        // kills: preferring the gap over a real receipt (case 1); refusing when
        // a named gap is present (case 2); inventing a witness when the layer
        // neither executed nor named a gap (case 3).
        assert_eq!(
            xmap22_execution_witness_or_named_gap(Some("e".repeat(64).as_str()), None, "XMAP14")
                .unwrap(),
            "e".repeat(64)
        );
        assert_eq!(
            xmap22_execution_witness_or_named_gap(
                Some("e".repeat(64).as_str()),
                Some("c".repeat(64).as_str()),
                "XMAP14",
            )
            .unwrap(),
            "e".repeat(64),
            "a layer that executed reports its receipt, not a gap"
        );
        assert_eq!(
            xmap22_execution_witness_or_named_gap(None, Some("c".repeat(64).as_str()), "XMAP14")
                .unwrap(),
            "c".repeat(64),
            "an excluded layer reports the witness of the gap that names it"
        );
        assert!(
            xmap22_execution_witness_or_named_gap(None, None, "XMAP14").is_err(),
            "no receipt and no gap is a refusal, never a fabricated witness"
        );

        for field in [
            "viewer",
            "xmap04",
            "xmap05",
            "xmap06",
            "xmap07",
            "xmap08",
            "xmap09",
            "xmap10",
            "xmap11",
            "xmap12",
            "xmap13_unit",
            "xmap13_counter",
            "xmap14",
            "xmap15",
            "xmap16_rail",
            "xmap17",
            "xmap19",
            "xmap20",
            "xmap21",
        ] {
            let mut contradictory = receipt.clone();
            let replacement = "b".repeat(64);
            match field {
                "viewer" => contradictory.viewer_state_sha256 = replacement,
                "xmap04" => contradictory.xmap04_terrain_execution_sha256 = replacement,
                "xmap05" => contradictory.xmap05_resource_execution_sha256 = replacement,
                "xmap06" => contradictory.xmap06_thin_borders_execution_sha256 = replacement,
                "xmap07" => contradictory.xmap07_water_execution_sha256 = replacement,
                "xmap08" => contradictory.xmap08_rivers_straits_execution_sha256 = replacement,
                "xmap09" => contradictory.xmap09_lighting_execution_sha256 = replacement,
                "xmap10" => contradictory.xmap10_postfx_receipt_sha256 = replacement,
                "xmap11" => contradictory.xmap11_tree_execution_sha256 = replacement,
                "xmap12" => contradictory.xmap12_execution_sha256 = replacement,
                "xmap13_unit" => contradictory.xmap13_unit_execution_sha256 = replacement,
                "xmap13_counter" => contradictory.xmap13_counter_execution_sha256 = replacement,
                "xmap14" => contradictory.xmap14_execution_sha256 = replacement,
                "xmap15" => contradictory.xmap15_execution_sha256 = replacement,
                "xmap16_rail" => contradictory.xmap16_rail_execution_sha256 = replacement,
                "xmap17" => contradictory.xmap17_execution_sha256 = replacement,
                "xmap19" => contradictory.xmap19_weather_execution_sha256 = replacement,
                "xmap20" => contradictory.xmap20_scene_receipt_sha256 = replacement,
                "xmap21" => contradictory.xmap21_execution_sha256 = replacement,
                _ => unreachable!(),
            }
            assert!(
                contradictory.seal().is_err(),
                "resealed contradictory {field} witness must fail"
            );
        }

        let mut value = serde_json::to_value(&receipt).unwrap();
        value["unexpected"] = serde_json::Value::Bool(true);
        assert!(serde_json::from_value::<ProductMapCompositionReceipt>(value).is_err());

        let mut invalid_supply = receipt.clone();
        invalid_supply.xmap16_supply = Xmap16SupplyCompositionDisposition::MapModeInactive {
            prepared_map_modes_sha256: "not-a-sha256".into(),
        };
        assert!(invalid_supply.validate().is_err());

        let mut invalid_hub = receipt.clone();
        invalid_hub.xmap16_hub = Xmap16HubCompositionDisposition::NoObserverCountry {
            viewer_state_sha256: "not-a-sha256".into(),
        };
        assert!(invalid_hub.validate().is_err());

        let mut recomputed_supply_witness = receipt.clone();
        recomputed_supply_witness.xmap16_supply =
            Xmap16SupplyCompositionDisposition::MapModeInactive {
                prepared_map_modes_sha256: "b".repeat(64),
            };
        assert!(recomputed_supply_witness.seal().is_err());

        let mut recomputed_hub_witness = receipt.clone();
        recomputed_hub_witness.xmap16_hub = Xmap16HubCompositionDisposition::NoObserverCountry {
            viewer_state_sha256: "b".repeat(64),
        };
        assert!(recomputed_hub_witness.seal().is_err());

        let mut recomputed_active_supply = receipt.clone();
        recomputed_active_supply.prepared_map_mode_composition = "supply".into();
        assert!(recomputed_active_supply.seal().is_err());

        let mut invalid_nested_foundation = receipt.clone();
        if let map_viewer::FamilyDisposition::Present { payload_schema, .. } =
            &mut invalid_nested_foundation.mandatory_family_execution_census[0]
                .foundation_disposition
        {
            payload_schema.clear();
        }
        invalid_nested_foundation.mandatory_family_execution_census_sha256 =
            snapshot::sha256::Sha256::hex(
                &serde_json::to_vec(&invalid_nested_foundation.mandatory_family_execution_census)
                    .unwrap(),
            );
        assert!(invalid_nested_foundation.seal().is_err());

        let mut missing_family = receipt.clone();
        missing_family.mandatory_family_execution_census.pop();
        missing_family.mandatory_family_execution_census_sha256 = snapshot::sha256::Sha256::hex(
            &serde_json::to_vec(&missing_family.mandatory_family_execution_census).unwrap(),
        );
        assert!(missing_family.seal().is_err());

        let mut category_mismatch = receipt.clone();
        category_mismatch.mandatory_family_execution_census[0].execution =
            Xmap22FamilyExecutionDisposition::MeasuredEmpty {
                execution_sha256: hash(),
                enumerator_witness_sha256: hash(),
            };
        category_mismatch.mandatory_family_execution_census_sha256 = snapshot::sha256::Sha256::hex(
            &serde_json::to_vec(&category_mismatch.mandatory_family_execution_census).unwrap(),
        );
        assert!(category_mismatch.seal().is_err());

        let mut pending = receipt.clone();
        pending.presented_to_product_target = false;
        pending.presentation_status = Xmap22PresentationStatus::WindowPendingPresent;
        let pending = pending.seal().expect("seal pending window receipt");
        for (outcome, expected_status, expected_presented) in [
            (
                crate::d3d::PresentOutcome::Presented,
                Xmap22PresentationStatus::WindowPresented,
                true,
            ),
            (
                crate::d3d::PresentOutcome::Skipped,
                Xmap22PresentationStatus::WindowSkipped,
                false,
            ),
            (
                crate::d3d::PresentOutcome::Occluded,
                Xmap22PresentationStatus::WindowOccluded,
                false,
            ),
        ] {
            let finalized = pending
                .clone()
                .finish_window_present(outcome)
                .expect("finalize window presentation receipt");
            assert_eq!(finalized.presentation_status, expected_status);
            assert_eq!(finalized.presented_to_product_target, expected_presented);
            finalized.validate().expect("validate finalized receipt");
        }
        assert!(receipt
            .clone()
            .finish_window_present(crate::d3d::PresentOutcome::Presented)
            .is_err());
    }

    #[test]
    fn all_shipping_routes_use_the_authoritative_xmap22_session() {
        let app = include_str!("app.rs");
        let app = &app[..app
            .find("mod product_production_d3d_tests {")
            .expect("test module marker")];
        let window = &app[app.find("pub fn run(").expect("window run entry")..];
        assert_eq!(
            window
                .matches("ProductMapCompositionSession::start_for_window_target(")
                .count(),
            1
        );
        assert_eq!(window.matches("composition.execute_on_target(").count(), 1);
        assert_eq!(window.matches("receipt.finish_window_present(").count(), 1);
        assert!(!window.contains("ProductMapFoundationSession::"));
        assert!(!window.contains("execute_water_frame("));
        assert!(!window.contains("execute_map_icon_buckets("));

        let presented = window
            .find("crate::d3d::PresentOutcome::Presented =>")
            .expect("presented transaction");
        let interaction_commit = window
            .find("effective_scripted_map_mode_ids = scripted_modes;")
            .expect("presented interaction commit");
        assert!(presented < interaction_commit);
        assert!(window[..presented].contains("pending_product_map_interaction = Some(("));

        let headless = include_str!("headless.rs");
        let headless_start = headless
            .find("fn dump_production_recording_frame(")
            .expect("headless product entry");
        let headless_end = headless_start
            + headless[headless_start..]
                .find("fn reject_production_dump_debug_inputs(")
                .expect("headless product boundary");
        let performance = include_str!("performance_bench.rs");
        let performance_start = performance
            .find("pub(crate) fn run(")
            .expect("benchmark product entry");
        let performance_end = performance_start
            + performance[performance_start..]
                .find("pub(crate) fn run_stress(")
                .expect("benchmark product boundary");
        for (route, constructor) in [
            (
                &headless[headless_start..headless_end],
                "ProductMapCompositionSession::start_for_headless(",
            ),
            (
                &performance[performance_start..performance_end],
                "ProductMapCompositionSession::start_for_benchmark(",
            ),
        ] {
            assert_eq!(route.matches(constructor).count(), 1);
            assert_eq!(route.matches("composition.execute(").count(), 1);
            assert!(!route.contains("execute_water_frame("));
            assert!(!route.contains("execute_map_icon_buckets("));
        }

        let census_start = app
            .find("let mut family_execution_witnesses = BTreeMap::new();")
            .expect("independent execution census start");
        let census_end = census_start
            + app[census_start..]
                .find("let mandatory_family_execution_census =")
                .expect("execution census seal");
        assert_eq!(
            app[census_start..census_end]
                .matches("family_execution_witnesses.insert(")
                .count(),
            map_viewer::MapVisualFamily::ALL.len()
        );

        let xmap07 = include_str!("../../map_viewer/src/production_features/xmap07.rs");
        let prepare = &xmap07[xmap07
            .find("impl ProductionMapFeatureAdapter for Xmap07WaterAdapter")
            .expect("XMAP07 product adapter")..];
        assert!(prepare.contains(".xmap07_snow_mud_runtime()"));
        assert!(!prepare.contains(".production_render()"));
        assert!(!prepare.contains("selected_present"));
        assert!(!prepare.contains("selected-Present"));

        let xmap04 = include_str!("../../map_viewer/src/production_features/xmap04.rs");
        let identity_start = xmap04
            .find("pub fn xmap04_replay_frame_identity_sha256")
            .expect("typed XMAP04 frame coordinate");
        let identity_end = identity_start
            + xmap04[identity_start..]
                .find("const TERRAIN_DOWNSTREAM_RESOURCE_OWNERS")
                .expect("XMAP04 frame coordinate boundary");
        let identity = &xmap04[identity_start..identity_end];
        assert!(!identity.contains("world_frame.raw()"));
        assert!(!identity.contains("production_render"));
        assert!(!identity.contains("selected"));
        assert!(identity.contains("world_frame.hour_index()"));
    }

    struct Xmap07IngressTestSource {
        hours: Vec<i64>,
        frame: replay_core::RawFrame,
    }

    impl replay_core::ReplaySource for Xmap07IngressTestSource {
        fn game_id(&self) -> &str {
            "xmap07-shipping-ingress"
        }

        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn frame(
            &self,
            hour_index: i64,
        ) -> std::result::Result<replay_core::RawFrame, replay_core::SourceError> {
            if hour_index != self.frame.hour_index {
                return Err(replay_core::SourceError::Open(format!(
                    "test source has no hour {hour_index}"
                )));
            }
            Ok(self.frame.clone())
        }
    }

    fn xmap07_ingress_test_carrier(hour: i64) -> replay_core::Xmap07PassiveSnowMudCarrier {
        let bytes = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let mut carrier = replay_core::Xmap07PassiveSnowMudCarrier {
            schema: replay_core::XMAP07_PASSIVE_SNOW_MUD_SCHEMA.into(),
            scope: replay_core::XMAP07_PASSIVE_SNOW_MUD_SCOPE.into(),
            executable_sha256: replay_core::A729_EXECUTABLE_SHA256.into(),
            texture_producer_rva: replay_core::SNOW_MUD_PRODUCER_FUNCTION_RVA,
            texture_producer_instructions_sha256:
                replay_core::SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256.into(),
            fade_updater_rva: replay_core::XMAP07_SNOW_MUD_FADE_UPDATER_RVA,
            fade_updater_instructions_sha256:
                replay_core::XMAP07_SNOW_MUD_FADE_UPDATER_INSTRUCTIONS_SHA256.into(),
            scene_copy_rva: replay_core::XMAP07_SNOW_MUD_SCENE_COPY_RVA,
            scene_copy_instructions_sha256:
                replay_core::XMAP07_SNOW_MUD_SCENE_COPY_INSTRUCTIONS_SHA256.into(),
            process_epoch: "process-1".into(),
            campaign_epoch: "campaign-1".into(),
            hook_epoch: "hook-1".into(),
            reset_epoch_ordinal: 1,
            capture_frontier_ordinal: 3,
            texture_observation_ordinal: 2,
            fade_observation_ordinal: 3,
            reset_hour: hour - 1,
            initial_snapshot_hour: hour - 1,
            payload_origin_hour: hour,
            texture_observation_hour: hour,
            fade_observation_hour: hour,
            displayed_hour: hour,
            carry_forward: false,
            reset_complete: true,
            initial_snapshot_complete: true,
            texture_complete: true,
            fade_complete: true,
            width: 2,
            height: 1,
            row_pitch: 8,
            bytes_per_texel: 4,
            format: replay_core::XMAP07_SNOW_MUD_FORMAT.into(),
            payload_sha256: snapshot::sha256::Sha256::hex(&bytes),
            payload_blob_sha256: replay_core::Xmap07PassiveSnowMudCarrier::payload_blob_sha256(
                &bytes,
            ),
            bytes,
            fade_f32_bits: 0.5f32.to_bits(),
            authority_sha256: String::new(),
        };
        carrier.authority_sha256 = carrier.authority_digest();
        carrier
    }

    fn xmap07_ingress_world_frame(
        extra: serde_json::Map<String, serde_json::Value>,
    ) -> Arc<WorldFrame> {
        let hour = 42;
        let mut rest = extra;
        rest.insert(
            replay_core::XMAP07_PASSIVE_SNOW_MUD_KEY.into(),
            serde_json::to_value(xmap07_ingress_test_carrier(hour)).unwrap(),
        );
        let timeline = replay_core::WorldTimeline::open(Box::new(Xmap07IngressTestSource {
            hours: vec![hour],
            frame: replay_core::RawFrame {
                hour_index: hour,
                date: Some("1936.1.2".into()),
                save_version: Some("a729".into()),
                schema_version: None,
                rest,
            },
        }))
        .expect("open XMAP07 ingress test timeline");
        timeline
            .frame(hour)
            .expect("decode XMAP07 ingress test frame")
    }

    #[test]
    fn xmap07_shipping_ingress_is_route_and_presentation_invariant_without_production_render() {
        let baseline = xmap07_ingress_world_frame(serde_json::Map::new());
        assert_eq!(
            baseline.production_render_status(),
            replay_core::ViewStatus::DomainAbsent
        );

        let variants = [
            serde_json::json!({
                "selected_country": "GER",
                "ui": {"open": true, "panel": "trade"},
                "camera": {"x": 100, "zoom": 0.2},
                "map_mode": "supply"
            }),
            serde_json::json!({
                "selected_country": "ENG",
                "ui": {"open": false},
                "camera": {"x": -900, "zoom": 5.0},
                "map_mode": "political"
            }),
            serde_json::json!({
                "production_render": {
                    "display_submit_index": "999",
                    "selected_present": {"candidate_id": "forbidden-decoy"}
                }
            }),
        ];
        let frames = std::iter::once(baseline.clone())
            .chain(
                variants
                    .into_iter()
                    .map(|value| xmap07_ingress_world_frame(value.as_object().unwrap().clone())),
            )
            .collect::<Vec<_>>();

        let expected = product_xmap07_shipping_ingress(ProductMapShippingRoute::Window, &baseline)
            .expect("window ingress");
        for route in [
            ProductMapShippingRoute::Window,
            ProductMapShippingRoute::Headless,
            ProductMapShippingRoute::Benchmark,
        ] {
            for frame in &frames {
                let actual = product_xmap07_shipping_ingress(route, frame)
                    .expect("shipping route admits passive XMAP07 carrier");
                assert_eq!(actual.displayed_hour, expected.displayed_hour);
                assert_eq!(actual.payload_blob_sha256, expected.payload_blob_sha256);
                assert_eq!(actual.authority_sha256, expected.authority_sha256);
            }
        }

        let missing = replay_core::WorldTimeline::open(Box::new(Xmap07IngressTestSource {
            hours: vec![42],
            frame: replay_core::RawFrame {
                hour_index: 42,
                date: Some("1936.1.2".into()),
                save_version: Some("a729".into()),
                schema_version: None,
                rest: serde_json::Map::new(),
            },
        }))
        .unwrap()
        .frame(42)
        .unwrap();
        assert!(
            product_xmap07_shipping_ingress(ProductMapShippingRoute::Headless, &missing).is_err()
        );
    }

    #[cfg(windows)]
    fn xmap_native_evidence_frame(
        provider: &dyn OwnerProvider,
        ordinal: usize,
    ) -> std::sync::Arc<replay_core::WorldFrame> {
        let hour = *provider
            .hours()
            .get(ordinal)
            .unwrap_or_else(|| panic!("native-proof fixture omitted hour ordinal {ordinal}"));
        provider
            .world_frame_at(hour)
            .unwrap_or_else(|error| panic!("decode native-proof hour {hour}: {error}"))
            .unwrap_or_else(|| panic!("native-proof provider omitted typed hour {hour}"))
    }

    #[test]
    fn xmap10_transport_seek_preserves_exact_constructor_zero_fold_in_constant_memory() {
        let main_scene = map_viewer::ProductionMainSceneTargetDesc {
            color_attachment_ordinal: 0,
            color_width: 320,
            color_height: 180,
            color_format: 10,
            color_samples: 1,
            color_quality: 0,
            depth_attachment_ordinal: 1,
            depth_width: 320,
            depth_height: 180,
            depth_format: 45,
            depth_samples: 1,
            depth_quality: 0,
        };
        let pre_seek = map_viewer::Xmap10PresentationRecurrenceStep {
            replay_frame_identity_sha256: snapshot::sha256::Sha256::hex(b"pre-seek"),
            frame_delta_seconds_f32_bits: (1.0f32 / 30.0).to_bits(),
            recorded_speed_raw: 4,
            recorded_paused: false,
            day_night_enabled: true,
        };
        let destination = map_viewer::Xmap10PresentationRecurrenceStep {
            replay_frame_identity_sha256: snapshot::sha256::Sha256::hex(b"destination"),
            frame_delta_seconds_f32_bits: (1.0f32 / 60.0).to_bits(),
            recorded_speed_raw: 2,
            recorded_paused: false,
            day_night_enabled: true,
        };
        let mut sought = map_viewer::Xmap10ViewerPresentationState::new().unwrap();
        let mut reference_timeline = Vec::new();
        for _ in 0..3 {
            reference_timeline.push(pre_seek.clone());
            xmap10_apply_bounded_presentation_step(&mut sought, &pre_seek, &main_scene, false)
                .unwrap();
        }
        reference_timeline.push(destination.clone());
        let patch =
            xmap10_apply_bounded_presentation_step(&mut sought, &destination, &main_scene, true)
                .unwrap();
        let mut expected = map_viewer::Xmap10ViewerPresentationState::new().unwrap();
        let expected_patch = expected
            .reconstruct_from_zero(&reference_timeline, &main_scene)
            .unwrap();
        assert_eq!(sought, expected);
        assert_eq!(patch, expected_patch);
        assert_eq!(sought.frame_sequence, 4);
    }

    #[test]
    fn xmap10_forward_jump_backward_seek_and_scrub_preserve_exact_bounded_fold() {
        let main_scene = map_viewer::ProductionMainSceneTargetDesc {
            color_attachment_ordinal: 0,
            color_width: 320,
            color_height: 180,
            color_format: 10,
            color_samples: 1,
            color_quality: 0,
            depth_attachment_ordinal: 1,
            depth_width: 320,
            depth_height: 180,
            depth_format: 45,
            depth_samples: 1,
            depth_quality: 0,
        };
        let scenarios = [
            ("initial", false, 1.0 / 60.0, 4, false),
            ("forward-jump", true, 0.20, 3, false),
            ("backward-seek", true, 0.01, 2, true),
            ("scrub", true, 0.07, 1, false),
            ("skipped-hour-playback", true, 0.25, 4, false),
        ];
        let mut state = map_viewer::Xmap10ViewerPresentationState::new().unwrap();
        let mut reference_timeline = Vec::new();
        for (label, discontinuity, delta, speed, paused) in scenarios {
            let step = map_viewer::Xmap10PresentationRecurrenceStep {
                replay_frame_identity_sha256: snapshot::sha256::Sha256::hex(label.as_bytes()),
                frame_delta_seconds_f32_bits: (delta as f32).to_bits(),
                recorded_speed_raw: speed,
                recorded_paused: paused,
                day_night_enabled: true,
            };
            reference_timeline.push(step.clone());
            let patch = xmap10_apply_bounded_presentation_step(
                &mut state,
                &step,
                &main_scene,
                discontinuity,
            )
            .unwrap_or_else(|error| panic!("{label}: {error}"));
            let mut independent = map_viewer::Xmap10ViewerPresentationState::new().unwrap();
            let independent_patch = independent
                .reconstruct_from_zero(&reference_timeline, &main_scene)
                .unwrap();
            assert_eq!(state, independent, "{label}");
            assert_eq!(patch, independent_patch, "{label}");
        }
        assert_eq!(state.frame_sequence, scenarios.len() as u64);
    }

    #[test]
    fn xmap10_discontinuity_forces_exact_temporal_resource_reset() {
        assert!(!xmap10_requires_temporal_resource_reset(false, false));
        assert!(xmap10_requires_temporal_resource_reset(true, false));
        assert!(xmap10_requires_temporal_resource_reset(false, true));
        assert!(xmap10_requires_temporal_resource_reset(true, true));
    }

    #[test]
    fn xmap10_rejected_present_discards_temporal_history_and_restores_recurrence() {
        let mut retained = map_viewer::Xmap10ViewerPresentationState::new().unwrap();
        let checkpoint_state = retained.clone();
        let scene = map_viewer::ProductionMainSceneTargetDesc {
            color_attachment_ordinal: 0,
            color_width: 320,
            color_height: 180,
            color_format: 10,
            color_samples: 1,
            color_quality: 0,
            depth_attachment_ordinal: 1,
            depth_width: 320,
            depth_height: 180,
            depth_format: 45,
            depth_samples: 1,
            depth_quality: 0,
        };
        retained
            .advance_step(
                &map_viewer::Xmap10PresentationRecurrenceStep {
                    replay_frame_identity_sha256: snapshot::sha256::Sha256::hex(b"rejected"),
                    frame_delta_seconds_f32_bits: (1.0f32 / 60.0).to_bits(),
                    recorded_speed_raw: 3,
                    recorded_paused: false,
                    day_night_enabled: true,
                },
                &scene,
            )
            .unwrap();
        let old_alpha = map_viewer::MapNameAlphaState::new(0.25, 0.75).unwrap();
        let mut native = ProductXmap10HandoffRuntime {
            temporal_receipt: Some(
                map_viewer::Xmap10TemporalAdaptationReceiptRef::seal(
                    retained.frame_sequence,
                    map_viewer::Xmap10TemporalAdaptationSource::EngineResourceReset {
                        reason: map_viewer::TemporalLuminanceResetReason::ResourcesAllocated,
                        main_scene_resource_identity_sha256: "a".repeat(64),
                        allocation_generation: 1,
                    },
                )
                .unwrap(),
            ),
            temporal_hour_index: Some(40),
            map_label_alpha: Some(map_viewer::MapNameAlphaState::new(0.9, 0.1).unwrap()),
            ..ProductXmap10HandoffRuntime::default()
        };
        reject_product_map_presentation(
            &mut retained,
            &mut native,
            ProductMapPresentationCheckpoint {
                presentation: checkpoint_state.clone(),
                map_label_alpha: Some(old_alpha),
            },
        );
        assert_eq!(retained, checkpoint_state);
        assert_eq!(native.map_label_alpha, Some(old_alpha));
        assert!(native.temporal_receipt.is_none());
        assert!(native.temporal_hour_index.is_none());
        assert!(native.main_scene.is_none());
        assert!(native.postfx.is_none());
        assert!(xmap10_requires_temporal_resource_reset(
            false,
            native.main_scene.is_none()
        ));
    }

    #[test]
    fn xui20_owns_middle_and_right_pointer_before_map_camera_drag() {
        for button in [MouseButton::Middle, MouseButton::Right] {
            assert!(product_map_camera_drag_allowed(button, false, false));
            assert!(!product_map_camera_drag_allowed(button, true, false));
            assert!(!product_map_camera_drag_allowed(button, false, true));
        }
        assert!(!product_map_camera_drag_allowed(
            MouseButton::Left,
            false,
            false
        ));
    }

    #[test]
    fn xmap20_authenticated_empty_status_is_sealed_and_tamper_evident() {
        let prepared_frame_sha256 = "a".repeat(64);
        let complete_enumeration_sha256 = "b".repeat(64);
        let main_scene_resource_identity_sha256 = "d".repeat(64);
        let main_scene_allocation_generation = 7;
        let pre_particle_scene_receipt_sha256 = "e".repeat(64);
        let scene_receipt_sha256 = snapshot::sha256::Sha256::hex(
            &serde_json::to_vec(&(
                "hoi4.xmap20.authenticated-empty-scene.v1",
                &prepared_frame_sha256,
                &complete_enumeration_sha256,
                &main_scene_resource_identity_sha256,
                main_scene_allocation_generation,
                &pre_particle_scene_receipt_sha256,
            ))
            .unwrap(),
        );
        let status = ProductXmap20ExecutionStatus::AuthenticatedEmpty {
            prepared_frame_sha256,
            complete_enumeration_sha256,
            main_scene_resource_identity_sha256,
            main_scene_allocation_generation,
            pre_particle_scene_receipt_sha256,
            scene_receipt_sha256,
        };
        status
            .validate()
            .expect("authenticated-empty status validates");

        let mut tampered = status;
        let ProductXmap20ExecutionStatus::AuthenticatedEmpty {
            complete_enumeration_sha256,
            ..
        } = &mut tampered
        else {
            unreachable!()
        };
        *complete_enumeration_sha256 = "c".repeat(64);
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn xmap10_temporal_continuity_accepts_only_same_or_next_hour() {
        assert!(xmap10_temporal_hour_is_consecutive(Some(40), 40));
        assert!(xmap10_temporal_hour_is_consecutive(Some(40), 41));
        assert!(!xmap10_temporal_hour_is_consecutive(Some(40), 39));
        assert!(!xmap10_temporal_hour_is_consecutive(Some(40), 42));
        assert!(!xmap10_temporal_hour_is_consecutive(None, 40));
    }

    #[cfg(windows)]
    #[test]
    fn xmap10_final_target_identity_tracks_real_retained_resource_and_readback() {
        let gfx = Gfx::create_offscreen(4, 3).expect("create native offscreen target");
        let generation = gfx.target_allocation_generation();
        let identity = unsafe {
            map_viewer::xmap10_final_rtv_resource_identity_sha256(
                gfx.device.as_raw(),
                gfx.rtv().unwrap().as_raw(),
                generation,
            )
            .unwrap()
        };
        gfx.clear_target([1.0, 0.0, 0.0, 1.0]).unwrap();
        let first = gfx.read_target().unwrap();
        assert!(first.chunks_exact(4).all(|pixel| pixel == [0, 0, 255, 255]));

        gfx.clear_target([0.0, 1.0, 0.0, 1.0]).unwrap();
        let second = gfx.read_target().unwrap();
        assert!(second
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 255, 0, 255]));
        let retained_identity = unsafe {
            map_viewer::xmap10_final_rtv_resource_identity_sha256(
                gfx.device.as_raw(),
                gfx.rtv().unwrap().as_raw(),
                generation,
            )
            .unwrap()
        };
        assert_eq!(retained_identity, identity);
        let stale_generation = unsafe {
            map_viewer::xmap10_final_rtv_resource_identity_sha256(
                gfx.device.as_raw(),
                gfx.rtv().unwrap().as_raw(),
                generation + 1,
            )
            .unwrap_err()
        };
        assert!(stale_generation
            .to_string()
            .contains("allocation tag/generation mismatch"));
    }

    #[cfg(windows)]
    #[test]
    #[cfg_attr(
        not(all(gate_hoi4_install, gate_xmap_native_evidence_replay)),
        ignore = "requires the matching the game install/user directory and an external-producer-certified replay carrying same-hour MAPC1/D18/FOW1, the XMAP06 runtime border-partition carrier, the passive global _xmap07_snow_mud_runtime texture/fade carrier, XMAP19 weather identity, and nonempty XMAP20 lifecycle authority; set XMAP_NATIVE_EVIDENCE_REPLAY to the replay and BATTLE_PLAN_USER_DIR when needed"
    )]
    fn gate_vanilla_native_xmap04_xmap07_xmap20_xmap10_chain_and_seek_refusal() {
        let game_root = replay_test_gates::require_vanilla_game_root();
        let user_data = replay_test_gates::require_hoi4_user_dir();
        let fixture = std::env::var_os("XMAP_NATIVE_EVIDENCE_REPLAY")
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .unwrap_or_else(|| {
                panic!(
                    "XMAP_NATIVE_EVIDENCE_REPLAY must name an external-producer-certified replay carrying same-hour MAPC1/D18/FOW1, the XMAP06 runtime border-partition carrier, the passive global _xmap07_snow_mud_runtime texture/fade carrier, XMAP19 weather identity, and nonempty XMAP20 lifecycle authority"
                )
            });
        let source = replay_core::open_recording(&fixture).unwrap_or_else(|error| {
            panic!("open native-proof fixture {}: {error}", fixture.display())
        });
        let timeline = replay_core::WorldTimeline::open(source)
            .expect("open native-proof replay-layer timeline");
        let provider = crate::timeline::RecordingTimeline::new_with_policy(timeline, None);
        assert!(
            provider.hours().len() >= 2,
            "native proof requires two replay hours"
        );
        assert_eq!(
            provider.hours()[1],
            provider.hours()[0]
                .checked_add(1)
                .expect("native-proof first hour overflowed"),
            "native proof requires two consecutive replay hours"
        );
        let product_paths = crate::install_paths::ProductPaths {
            game_install: game_root,
            user_data,
        };
        let (resolver, content_admission) =
            crate::content_compatibility::bind_provider_content(&provider, &product_paths)
                .expect("bind native-proof replay through the production content path");
        let playback_ui_profile = crate::install_paths::playback_ui_profile_from_paths(
            &product_paths,
            &resolver,
            crate::win32::actual_client_input_variant(),
        )
        .expect("load native-proof playback UI profile");
        let manifest = provider
            .content_manifest()
            .expect("native-proof replay has no recorded content manifest")
            .clone();

        let (map_width, map_height) =
            map_viewer::foundation_map_dimensions(&resolver, &content_admission)
                .expect("resolve native-proof map dimensions");
        const WIDTH: u32 = 320;
        const HEIGHT: u32 = 180;
        let camera_state = map_viewer::ViewerMapCameraState::from_admitted_content(
            &resolver,
            &content_admission,
            map_width,
            map_height,
            WIDTH,
            HEIGHT,
            map_viewer::DEFAULT_VIEWER_CAMERA_SPEED_SETTING,
        )
        .expect("construct native-proof viewer camera");
        let product_camera =
            Camera::from_viewer_state(camera_state).expect("construct native-proof product camera");
        let mut viewer = current_camera_viewer_state(
            &product_camera,
            None,
            &map_viewer::ViewerMapModeSelection::default(),
            0,
            &[],
            0.0,
            0.0,
        )
        .expect("construct native-proof viewer state");
        let terrain_lod_options =
            crate::install_paths::playback_terrain_lod_options_from_paths(&product_paths)
                .expect("load selected-profile native-proof map rendering options");
        let mut foundation = crate::validate::ProductMapFoundationSession::start(
            &resolver,
            &content_admission,
            &manifest,
            terrain_lod_options,
        )
        .expect("start native-proof product foundation");
        let (_topology, assigned_schedule) =
            crate::validate::product_foundation_assigned_stage_plan()
                .expect("build native-proof assigned-stage plan");
        let tree_schedule = assigned_schedule
            .require(
                map_viewer::XMAP11_TREE_STAGE_NAME,
                map_viewer::XMAP11_TREE_OPERATION_NAME,
            )
            .expect("locate native-proof XMAP11 assigned stage")
            .clone();

        let gfx = Gfx::create_offscreen(WIDTH, HEIGHT)
            .expect("native XMAP chain requires a working D3D11 device");
        let mut terrain = ProductTerrainHandoffRuntime::default();
        let mut political = ProductPoliticalHandoffRuntime::default();
        let mut lighting = ProductLightingHandoffRuntime::default();
        let mut trees = ProductTreeHandoffRuntime::default();
        let mut water = ProductWaterHandoffRuntime::default();
        let mut thin_borders = ProductThinBorderHandoffRuntime::default();
        let mut rivers_straits = ProductRiversStraitsHandoffRuntime::default();
        let mut xmap10 = ProductXmap10HandoffRuntime::default();
        let mut xmap12 = ProductXmap12HandoffRuntime::default();
        let mut xmap13_units = ProductXmap13UnitRuntime::default();
        let mut orders = ProductOrdersHandoffRuntime::default();
        let mut rail = ProductRailHandoffRuntime::default();
        let mut furniture_render =
            FurnitureRenderPass::new(&gfx, &resolver, playback_ui_profile.localisation_language())
                .expect("retain native-proof XMAP21 furniture resources");
        let mut presentation = map_viewer::Xmap10ViewerPresentationState::new()
            .expect("construct native-proof presentation state");
        let mut readbacks = Vec::new();
        let mut completed_receipts = Vec::new();
        let mut present_xmap20_frames = 0usize;
        let mut last_recurrence_step = None;
        const SENTINEL_BGRA: [u8; 4] = [255, 0, 255, 255];

        for ordinal in [0usize, 1] {
            let frame_delta_seconds = 1.0f32 / 60.0;
            let animation_time =
                f32::from_bits(viewer.animation_time_f32_bits) + frame_delta_seconds;
            viewer.animation_time_f32_bits = animation_time.to_bits();
            viewer.frame_delta_seconds_f32_bits = frame_delta_seconds.to_bits();
            let world = xmap_native_evidence_frame(&provider, ordinal);
            let xmap12_inputs = crate::validate::authenticated_xmap12_frame_inputs(&world)
                .unwrap_or_else(|error| {
                    panic!("admit native-proof XMAP12 inputs for frame {ordinal}: {error}")
                });
            let prepared = foundation
                .prepare_with_xmap12_inputs(
                    &world,
                    &viewer,
                    &manifest,
                    xmap12_inputs.building_visual.as_ref(),
                    xmap12_inputs.fixed_runtime.as_ref(),
                )
                .unwrap_or_else(|error| panic!("prepare native-proof frame {ordinal}: {error}"));
            require_current_camera_foundation(&product_camera, &prepared.envelope)
                .expect("native-proof camera join");
            require_current_terrain_foundation(&prepared).expect("native-proof XMAP04 payload");
            require_current_trees_foundation(&prepared, &tree_schedule)
                .expect("native-proof XMAP11 payload");
            require_current_water_foundation(&prepared).expect("native-proof XMAP07 payload");
            require_current_thin_borders_foundation(&prepared)
                .expect("native-proof XMAP06 foundation join")
                .expect("native-proof XMAP06 runtime partition");

            terrain
                .retain_frame(&gfx, &resolver, &prepared)
                .expect("retain native-proof XMAP04");
            political
                .retain_frame(&gfx, &prepared)
                .expect("retain native-proof XMAP05");
            lighting
                .retain_frame(&gfx, &product_camera, &prepared)
                .expect("retain native-proof XMAP09");
            trees
                .retain_frame(&gfx, &resolver, &content_admission, &prepared)
                .expect("retain native-proof XMAP11");
            xmap13_units
                .retain_owned_resources(
                    &gfx,
                    &resolver,
                    &content_admission,
                    &manifest,
                    &prepared,
                    terrain_lod_options.texture_quality,
                    map_width,
                    map_height,
                )
                .expect("retain native-proof XMAP13");
            water
                .retain_frame(&gfx, &resolver, &prepared)
                .expect("retain native-proof XMAP07");
            thin_borders
                .retain_frame(&gfx, &resolver, &prepared)
                .expect("retain native-proof XMAP06")
                .expect("native-proof XMAP06 native receipt");
            rivers_straits
                .retain_frame(&gfx, &resolver, &prepared)
                .expect("retain native-proof XMAP08");
            orders
                .retain_frame(&gfx, &resolver, &prepared)
                .expect("retain native-proof XMAP15");
            rail.retain_frame(&gfx, &resolver, &prepared)
                .expect("retain native-proof XMAP16");
            terrain
                .bind_political_dependencies(&gfx, &political)
                .expect("bind native-proof XMAP05 to XMAP04");
            terrain
                .bind_water_dependencies(&gfx, &water)
                .expect("bind native-proof XMAP07 to XMAP04");

            let targets = map_viewer::Xmap10TargetPlan::new(WIDTH, HEIGHT)
                .expect("native-proof XMAP10 target plan");
            let main_scene =
                map_viewer::production_main_scene_inputs_from_viewer(&viewer, &targets)
                    .expect("native-proof MainScene inputs");
            let step = map_viewer::Xmap10PresentationRecurrenceStep::from_world_frame(
                &world,
                &prepared.terrain.replay_frame_identity_sha256,
                frame_delta_seconds,
                true,
            )
            .expect("native-proof presentation recurrence step");
            let presentation_patch = xmap10_apply_bounded_presentation_step(
                &mut presentation,
                &step,
                &main_scene.desc,
                false,
            )
            .expect("advance native-proof presentation state");
            last_recurrence_step = Some(step.clone());
            let map_clock = map_viewer::Xmap05AuthenticatedMapClock::from_viewer_presentation(
                prepared.envelope.hour_index,
                &viewer,
                &presentation,
            )
            .expect("native-proof authenticated map clock");
            let political_patch = prepared
                .political
                .scene_cb0_patch(
                    viewer.camera.as_ref().expect("native-proof viewer camera"),
                    &map_clock,
                )
                .expect("native-proof XMAP05 SceneCB0 patch");

            gfx.clear_target([1.0, 0.0, 1.0, 1.0])
                .expect("clear native-proof final target to sentinel");
            unsafe {
                xmap10
                    .execute_water_frame(
                        &gfx,
                        &resolver,
                        &content_admission,
                        &manifest,
                        &world,
                        playback_ui_profile.localisation_language(),
                        frame_delta_seconds,
                        false,
                        &prepared,
                        &viewer,
                        &political_patch,
                        &presentation_patch,
                        main_scene.desc,
                        main_scene.color,
                        main_scene.depth,
                        main_scene.producer_sha256,
                        &mut terrain,
                        &political,
                        &mut lighting,
                        &mut trees,
                        &tree_schedule,
                        &mut xmap12,
                        &mut xmap13_units,
                        &mut water,
                        &mut thin_borders,
                        &mut rivers_straits,
                        &mut orders,
                        &mut rail,
                        &mut furniture_render,
                    )
                    .unwrap_or_else(|error| {
                        panic!(
                            "execute native XMAP04/XMAP07/XMAP20/XMAP10 frame {ordinal}: {error}"
                        )
                    });
            }
            let xmap20_status = xmap10
                .xmap20_effects
                .as_ref()
                .expect("native-proof XMAP20 completion");
            xmap20_status
                .validate()
                .expect("validate native-proof XMAP20 receipt");
            if let ProductXmap20ExecutionStatus::Present(receipt) = xmap20_status {
                assert!(receipt.particle_draw_count > 0);
                present_xmap20_frames += 1;
            }
            let completed = xmap10
                .completed_postfx
                .as_ref()
                .expect("native-proof XMAP10 completion");
            completed
                .validate()
                .expect("validate native-proof XMAP10 receipt chain");
            assert_eq!(
                xmap10
                    .pre_particle_scene
                    .as_ref()
                    .expect("native-proof preparticle scene")
                    .scene_cb0
                    .sha256,
                xmap10
                    .scene_cb0s
                    .iter()
                    .find(|scene| scene.prepared().frame_identity.render_view_mask == 1)
                    .expect("native-proof main SceneCB0")
                    .prepared()
                    .payload_sha256
            );
            completed_receipts.push(completed.postfx_frame.packet_frame_receipt_sha256.clone());
            let pixels = gfx.read_target().expect("read native-proof final target");
            assert_eq!(pixels.len(), usize::try_from(WIDTH * HEIGHT * 4).unwrap());
            assert!(
                pixels.chunks_exact(4).any(|pixel| pixel != SENTINEL_BGRA),
                "native XMAP04/XMAP07/XMAP20/XMAP10 chain left the final target sentinel untouched"
            );
            readbacks.push(snapshot::sha256::Sha256::hex(&pixels));
        }

        assert_eq!(completed_receipts.len(), 2);
        assert!(completed_receipts.iter().all(|receipt| receipt.len() == 64));
        assert!(readbacks.iter().all(|receipt| receipt.len() == 64));
        assert!(
            present_xmap20_frames > 0,
            "native proof executed no XMAP20 particle draw"
        );
        assert_ne!(completed_receipts[0], completed_receipts[1]);
        assert_eq!(presentation.frame_sequence, 2);
        assert_eq!(
            presentation.replay_frame_identity_sha256,
            last_recurrence_step
                .as_ref()
                .expect("native-proof recurrence destination")
                .replay_frame_identity_sha256
        );

        let seek_world = xmap_native_evidence_frame(&provider, 0);
        let seek_delta_seconds = 1.0f32 / 60.0;
        let seek_animation_time =
            f32::from_bits(viewer.animation_time_f32_bits) + seek_delta_seconds;
        viewer.animation_time_f32_bits = seek_animation_time.to_bits();
        viewer.frame_delta_seconds_f32_bits = seek_delta_seconds.to_bits();
        let seek_step = map_viewer::Xmap10PresentationRecurrenceStep::from_world_frame(
            &seek_world,
            &map_viewer::xmap04_replay_frame_identity_sha256(&seek_world)
                .expect("derive native-proof seek frame identity"),
            seek_delta_seconds,
            true,
        )
        .expect("construct native-proof seek recurrence step");
        let seek_targets = map_viewer::Xmap10TargetPlan::new(WIDTH, HEIGHT)
            .expect("native-proof seek target plan");
        let seek_main_scene =
            map_viewer::production_main_scene_inputs_from_viewer(&viewer, &seek_targets)
                .expect("native-proof seek MainScene inputs");
        let mut expected = presentation.clone();
        let seek_patch = xmap10_apply_bounded_presentation_step(
            &mut presentation,
            &seek_step,
            &seek_main_scene.desc,
            true,
        )
        .expect("reconstruct native-proof presentation after backward seek");
        let expected_patch = expected
            .advance_step(&seek_step, &seek_main_scene.desc)
            .expect("advance native-proof backward-seek destination");
        assert_eq!(presentation, expected);
        assert_eq!(presentation.frame_sequence, 3);
        assert_eq!(seek_patch, expected_patch);
    }

    #[test]
    fn production_entry_plan_contains_all_promoted_map_stages() {
        let (topology, schedule) =
            crate::validate::product_foundation_assigned_stage_plan().unwrap();
        schedule.validate_against(&topology).unwrap();

        let border_early = schedule
            .require(
                map_viewer::XMAP06_EARLY_STAGE_NAME,
                map_viewer::XMAP06_EARLY_OPERATION_NAME,
            )
            .unwrap();
        let border_late = schedule
            .require(
                map_viewer::XMAP06_LATE_STAGE_NAME,
                map_viewer::XMAP06_LATE_OPERATION_NAME,
            )
            .unwrap();
        let lighting_prepare = schedule
            .require(
                map_viewer::XMAP09_PREPARE_STAGE_NAME,
                map_viewer::XMAP09_PREPARE_OPERATION_NAME,
            )
            .unwrap();
        let shadow_execute = schedule
            .require(
                map_viewer::XMAP09_EXECUTE_STAGE_NAME,
                map_viewer::XMAP09_EXECUTE_OPERATION_NAME,
            )
            .unwrap();
        let weather_prepare = schedule
            .require(
                map_viewer::XMAP19_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP19_ASSIGNED_OPERATION_NAME,
            )
            .unwrap();
        let rail_execute = schedule
            .require(
                map_viewer::XMAP16_RAIL_STAGE_NAME,
                map_viewer::XMAP16_RAIL_OPERATION_ID,
            )
            .unwrap();
        let supply_execute = schedule
            .require(
                map_viewer::XMAP16_SUPPLY_ARROW_OPERATION_ID,
                map_viewer::XMAP16_SUPPLY_ARROW_OPERATION_NAME,
            )
            .unwrap();
        let hub_execute = schedule
            .require(
                map_viewer::XMAP16_HUB_STAGE_NAME,
                map_viewer::XMAP16_HUB_OPERATION_NAME,
            )
            .unwrap();

        let water_surface = schedule
            .require(
                map_viewer::XMAP07_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP07_ASSIGNED_OPERATION_NAME,
            )
            .unwrap();
        let trees = schedule
            .require(
                map_viewer::XMAP11_TREE_STAGE_NAME,
                map_viewer::XMAP11_TREE_OPERATION_NAME,
            )
            .unwrap();
        let rivers = schedule
            .require(
                map_viewer::XMAP08_RIVER_STAGE_NAME,
                map_viewer::XMAP08_RIVER_OPERATION_ID,
            )
            .unwrap();
        let straits = schedule
            .require(
                map_viewer::XMAP08_STRAIT_STAGE_NAME,
                map_viewer::XMAP08_STRAIT_OPERATION_ID,
            )
            .unwrap();
        let unit_entities = schedule
            .require(
                map_viewer::XMAP13_UNIT_ENTITY_STAGE_NAME,
                map_viewer::XMAP13_UNIT_ENTITY_OPERATION_NAME,
            )
            .unwrap();
        let counter_quads = schedule
            .require(
                map_viewer::XMAP13_COUNTER_QUAD_STAGE_NAME,
                map_viewer::XMAP13_COUNTER_QUAD_OPERATION_NAME,
            )
            .unwrap();
        let combat_indicators = schedule
            .require(
                map_viewer::XMAP14_COMBAT_STAGE_NAME,
                map_viewer::XMAP14_COMBAT_OPERATION_NAME,
            )
            .unwrap();
        let orders = schedule
            .require(
                map_viewer::XMAP15_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP15_ASSIGNED_OPERATION_NAME,
            )
            .unwrap();
        let particles = schedule
            .require(
                map_viewer::XMAP20_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP20_ASSIGNED_OPERATION_NAME,
            )
            .unwrap();
        let labels_prepare = schedule
            .require(
                map_viewer::XMAP17_PREPARE_STAGE,
                map_viewer::XMAP17_PREPARE_OPERATION,
            )
            .unwrap();
        let labels_execute = schedule
            .require(
                map_viewer::XMAP17_EXECUTE_STAGE,
                map_viewer::XMAP17_EXECUTE_OPERATION,
            )
            .unwrap();
        let furniture_state = schedule
            .require("map_mode", "map_mode.furniture_state")
            .unwrap();
        let furniture_prepare = schedule
            .require(
                map_viewer::XMAP21_PREPARE_STAGE_NAME,
                map_viewer::XMAP21_PREPARE_OPERATION_NAME,
            )
            .unwrap();
        let furniture_execute = schedule
            .require(
                map_viewer::XMAP21_EXECUTE_STAGE_NAME,
                map_viewer::XMAP21_EXECUTE_OPERATION_NAME,
            )
            .unwrap();

        assert_eq!(topology.ordered_stages().len(), 22);
        assert_eq!(topology.ordered_edges().len(), 18);
        assert!(border_early.assigned_ordinal < rivers.assigned_ordinal);
        assert!(rivers.assigned_ordinal < straits.assigned_ordinal);
        assert!(straits.assigned_ordinal < rail_execute.assigned_ordinal);
        assert!(rail_execute.assigned_ordinal < trees.assigned_ordinal);
        assert!(trees.assigned_ordinal < water_surface.assigned_ordinal);
        assert!(water_surface.assigned_ordinal < border_late.assigned_ordinal);
        assert!(border_late.assigned_ordinal < supply_execute.assigned_ordinal);
        assert!(supply_execute.assigned_ordinal < particles.assigned_ordinal);
        assert!(supply_execute.assigned_ordinal < labels_prepare.assigned_ordinal);
        assert!(labels_prepare.assigned_ordinal < labels_execute.assigned_ordinal);
        assert!(labels_execute.assigned_ordinal < particles.assigned_ordinal);
        assert!(particles.assigned_ordinal < hub_execute.assigned_ordinal);
        assert!(orders.assigned_ordinal < counter_quads.assigned_ordinal);
        assert!(counter_quads.assigned_ordinal < combat_indicators.assigned_ordinal);
        assert!(combat_indicators.assigned_ordinal < hub_execute.assigned_ordinal);
        assert!(furniture_state.assigned_ordinal < furniture_prepare.assigned_ordinal);
        assert!(furniture_prepare.assigned_ordinal < furniture_execute.assigned_ordinal);
        assert!(lighting_prepare.assigned_ordinal < shadow_execute.assigned_ordinal);
        assert_eq!(rivers.assigned_ordinal, 2);
        assert_eq!(straits.assigned_ordinal, 3);
        assert_eq!(rail_execute.assigned_ordinal, 4);
        assert_eq!(trees.assigned_ordinal, 5);
        assert_eq!(water_surface.assigned_ordinal, 6);
        assert_eq!(border_late.assigned_ordinal, 7);
        assert_eq!(supply_execute.assigned_ordinal, 8);
        assert_eq!(weather_prepare.assigned_ordinal, 11);
        assert_eq!(unit_entities.assigned_ordinal, 12);
        assert_eq!(orders.assigned_ordinal, 13);
        assert_eq!(counter_quads.assigned_ordinal, 14);
        assert_eq!(combat_indicators.assigned_ordinal, 15);
        assert_eq!(labels_prepare.assigned_ordinal, 16);
        assert_eq!(labels_execute.assigned_ordinal, 17);
        assert_eq!(particles.assigned_ordinal, 18);
        assert_eq!(hub_execute.assigned_ordinal, 19);
        assert_eq!(furniture_state.assigned_ordinal, 20);
        assert_eq!(furniture_prepare.assigned_ordinal, 21);
        assert_eq!(furniture_execute.assigned_ordinal, 22);
    }

    #[test]
    fn product_label_draw_executes_after_arrows_and_before_particles() {
        let source = include_str!("app.rs");
        let production = &source[..source
            .find("mod product_production_d3d_tests {")
            .expect("test module marker")];
        let supply = production
            .find("let supply_executions = if xmap16_present")
            .expect("XMAP16 supply execution");
        let labels = production[supply..]
            .find(".draw_prepared_product(")
            .map(|offset| supply + offset)
            .expect("XMAP17 product draw");
        let alpha_commit = production[labels..]
            .find("self.map_label_alpha = Some(next_label_alpha)")
            .map(|offset| labels + offset)
            .expect("XMAP17 alpha commit");
        let receipt_commit = production[alpha_commit..]
            .find("self.map_label_draw = Some(label_draw)")
            .map(|offset| alpha_commit + offset)
            .expect("XMAP17 receipt commit");
        let particles = production[receipt_commit..]
            .find("let dependencies = replay_core::Xmap20DependencyReceipts")
            .map(|offset| receipt_commit + offset)
            .expect("XMAP20 dependency admission");
        assert!(supply < labels);
        assert!(labels < alpha_commit && alpha_commit < receipt_commit);
        assert!(receipt_commit < particles);
        assert!(production.contains("self.xmap10.map_label_execution_complete()"));
    }

    #[test]
    fn xui16_wheel_routes_exact_notches_in_both_directions_for_each_panel() {
        use game_ui::occupation_compositor::{
            OccupationInputRegion, OccupationPanel, OccupationViewerState,
        };
        for panel in [
            OccupationPanel::Territories,
            OccupationPanel::GarrisonActivities,
            OccupationPanel::GarrisonEquipment,
        ] {
            let mut viewer = OccupationViewerState {
                panel,
                territory_scroll_row: 5,
                activity_scroll_row: 5,
                equipment_scroll_row: 5,
                ..Default::default()
            };
            let mut accumulator = OccupationWheelAccumulator::default();
            let region = Some(OccupationInputRegion {
                panel,
                panel_rect: [0, 0, 10, 10],
                scroll_rect: [0, 0, 10, 10],
            });
            assert!(route_occupation_wheel(
                &mut viewer,
                &mut accumulator,
                region,
                120,
                5,
                5,
            ));
            let row = match panel {
                OccupationPanel::Territories => viewer.territory_scroll_row,
                OccupationPanel::GarrisonActivities => viewer.activity_scroll_row,
                OccupationPanel::GarrisonEquipment => viewer.equipment_scroll_row,
                OccupationPanel::Closed => unreachable!(),
            };
            assert_eq!(row, 4, "positive/up wheel decrements the logical row");
            assert!(route_occupation_wheel(
                &mut viewer,
                &mut accumulator,
                region,
                -120,
                5,
                5,
            ));
            let row = match panel {
                OccupationPanel::Territories => viewer.territory_scroll_row,
                OccupationPanel::GarrisonActivities => viewer.activity_scroll_row,
                OccupationPanel::GarrisonEquipment => viewer.equipment_scroll_row,
                OccupationPanel::Closed => unreachable!(),
            };
            assert_eq!(row, 5, "negative/down wheel increments the logical row");
        }
    }

    #[test]
    fn xui16_wheel_accumulates_partial_deltas_per_panel_and_closed_is_noop() {
        use game_ui::occupation_compositor::{
            OccupationInputRegion, OccupationPanel, OccupationViewerState,
        };
        let mut viewer = OccupationViewerState {
            panel: OccupationPanel::Territories,
            ..Default::default()
        };
        let mut accumulator = OccupationWheelAccumulator::default();
        let territory = Some(OccupationInputRegion {
            panel: OccupationPanel::Territories,
            panel_rect: [0, 0, 10, 10],
            scroll_rect: [0, 0, 10, 10],
        });
        let activities = Some(OccupationInputRegion {
            panel: OccupationPanel::GarrisonActivities,
            panel_rect: [0, 0, 10, 10],
            scroll_rect: [0, 0, 10, 10],
        });
        assert!(route_occupation_wheel(
            &mut viewer,
            &mut accumulator,
            territory,
            -60,
            5,
            5,
        ));
        assert_eq!(viewer.territory_scroll_row, 0);
        viewer.panel = OccupationPanel::GarrisonActivities;
        assert!(route_occupation_wheel(
            &mut viewer,
            &mut accumulator,
            activities,
            -60,
            5,
            5,
        ));
        assert_eq!(viewer.activity_scroll_row, 0);
        viewer.panel = OccupationPanel::Territories;
        assert!(route_occupation_wheel(
            &mut viewer,
            &mut accumulator,
            territory,
            -60,
            5,
            5,
        ));
        assert_eq!(viewer.territory_scroll_row, 1);
        viewer.panel = OccupationPanel::GarrisonActivities;
        assert!(route_occupation_wheel(
            &mut viewer,
            &mut accumulator,
            activities,
            -60,
            5,
            5,
        ));
        assert_eq!(viewer.activity_scroll_row, 1);

        viewer.panel = OccupationPanel::Closed;
        let before = accumulator.clone();
        assert!(!route_occupation_wheel(
            &mut viewer,
            &mut accumulator,
            activities,
            -120,
            5,
            5,
        ));
        assert_eq!(accumulator, before);
    }

    #[test]
    fn xui16_wheel_refuses_outside_stale_or_missing_authored_regions() {
        use game_ui::occupation_compositor::{
            OccupationInputRegion, OccupationPanel, OccupationViewerState,
        };
        let mut viewer = OccupationViewerState {
            panel: OccupationPanel::Territories,
            territory_scroll_row: 2,
            ..Default::default()
        };
        let mut accumulator = OccupationWheelAccumulator::default();
        let territory = Some(OccupationInputRegion {
            panel: OccupationPanel::Territories,
            panel_rect: [0, 0, 100, 100],
            scroll_rect: [10, 20, 30, 40],
        });
        let stale = Some(OccupationInputRegion {
            panel: OccupationPanel::GarrisonActivities,
            panel_rect: [0, 0, 100, 100],
            scroll_rect: [10, 20, 30, 40],
        });
        for (region, point) in [
            (None, (20, 30)),
            (stale, (20, 30)),
            (territory, (9, 30)),
            (territory, (40, 30)),
            (territory, (20, 19)),
            (territory, (20, 60)),
        ] {
            assert!(!route_occupation_wheel(
                &mut viewer,
                &mut accumulator,
                region,
                -120,
                point.0,
                point.1,
            ));
        }
        assert_eq!(viewer.territory_scroll_row, 2);
        assert_eq!(accumulator, OccupationWheelAccumulator::default());
    }

    #[test]
    fn xui16_keyboard_controls_require_open_country_joined_panel() {
        use game_ui::occupation_compositor::OccupationPanel;
        let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();
        assert!(!occupation_controls_active(&viewer, Some("GER")));
        viewer.occupation.panel = OccupationPanel::Territories;
        assert!(!occupation_controls_active(&viewer, Some("GER")));
        viewer.occupation.selected_country = "GER".into();
        assert!(occupation_controls_active(&viewer, Some("GER")));
        assert!(!occupation_controls_active(&viewer, Some("FRA")));
        assert!(!occupation_controls_active(&viewer, None));
    }

    #[test]
    fn resize_cancels_app_captures_for_same_batch_move_and_retry() {
        let mut drag = Some("old-size-map-drag");
        let mut scrub_drag = true;
        let mut checkpoint_drag = drag;
        let mut checkpoint_scrub_drag = scrub_drag;

        cancel_app_pointer_capture_for_resize(&mut drag, &mut scrub_drag);
        cancel_app_pointer_capture_for_resize(
            &mut checkpoint_drag,
            &mut checkpoint_scrub_drag,
        );

        assert!(drag.is_none());
        assert!(!scrub_drag);

        drag = checkpoint_drag;
        scrub_drag = checkpoint_scrub_drag;
        assert!(drag.is_none());
        assert!(!scrub_drag);
    }

    #[test]
    fn resize_refreshes_viewport_for_new_same_batch_input() {
        let mut screen_w = 1_920.0;
        let mut screen_h = 1_080.0;

        refresh_cached_input_viewport(&mut screen_w, &mut screen_h, 800, 400);
        assert_eq!((screen_w, screen_h), (800.0, 400.0));

        let same_batch_drag_fraction = 40.0 / screen_h;
        assert_eq!(same_batch_drag_fraction, 0.1);
    }

    #[test]
    fn xui16_pointer_ownership_uses_the_last_presented_authored_extent() {
        use game_ui::occupation_compositor::{OccupationInputRegion, OccupationPanel};
        let current = Some(OccupationInputRegion {
            panel: OccupationPanel::Territories,
            panel_rect: [10, 20, 30, 40],
            scroll_rect: [15, 25, 10, 20],
        });
        let stale = Some(OccupationInputRegion {
            panel: OccupationPanel::GarrisonActivities,
            panel_rect: [10, 20, 30, 40],
            scroll_rect: [15, 25, 10, 20],
        });
        assert!(occupation_owns_pointer(current, 10, 20));
        assert!(occupation_owns_pointer(current, 39, 59));
        assert!(!occupation_owns_pointer(current, 40, 59));
        assert!(!occupation_owns_pointer(current, 39, 60));
        assert!(
            occupation_owns_pointer(stale, 20, 30),
            "the last presented panel keeps ownership while a replacement frame is pending"
        );
        assert!(!occupation_owns_pointer(None, 20, 30));
    }

    #[test]
    fn xui16_applied_scroll_feedback_replaces_overrun_without_touching_other_panels() {
        use game_ui::occupation_compositor::{OccupationPanel, OccupationViewerState};
        let mut viewer = OccupationViewerState {
            panel: OccupationPanel::GarrisonEquipment,
            territory_scroll_row: 7,
            activity_scroll_row: 8,
            equipment_scroll_row: usize::MAX,
            ..Default::default()
        };
        apply_occupation_scroll_feedback(&mut viewer, Some(3));
        assert_eq!(viewer.territory_scroll_row, 7);
        assert_eq!(viewer.activity_scroll_row, 8);
        assert_eq!(viewer.equipment_scroll_row, 3);

        viewer.panel = OccupationPanel::Closed;
        apply_occupation_scroll_feedback(&mut viewer, Some(0));
        assert_eq!(viewer.equipment_scroll_row, 3);
    }

    #[test]
    fn windowed_production_recording_refuses_debug_bundle_and_capture_overlay() {
        use crate::blockers::{reason, BlockerId};
        reject_production_recording_debug_surfaces(false, true, true)
            .expect("debug modes keep their debug inputs");
        reject_production_recording_debug_surfaces(true, false, false)
            .expect("plain production recording is admitted");
        let err = reject_production_recording_debug_surfaces(true, true, false)
            .expect_err("bundle must be refused");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingBundle));
        let err = reject_production_recording_debug_surfaces(true, false, true)
            .expect_err("capture overlay must be refused");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingCapture));
    }
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
    use std::sync::atomic::AtomicUsize;
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::Arc;

    #[test]
    fn exact_wheel_accumulator_retains_fraction_and_resets_on_reversal() {
        let mut remainder = 0;
        assert_eq!(
            accumulate_engine_wheel_notches(&mut remainder, 40).expect("first fraction"),
            0
        );
        assert_eq!(remainder, 40);
        assert_eq!(
            accumulate_engine_wheel_notches(&mut remainder, 80).expect("complete notch"),
            1
        );
        assert_eq!(remainder, 0);

        assert_eq!(
            accumulate_engine_wheel_notches(&mut remainder, -250).expect("two negative notches"),
            -2
        );
        assert_eq!(remainder, -10);
        assert_eq!(
            accumulate_engine_wheel_notches(&mut remainder, 119).expect("reversal resets"),
            0
        );
        assert_eq!(remainder, 119);
        assert_eq!(
            accumulate_engine_wheel_notches(&mut remainder, 1).expect("positive notch"),
            1
        );
        assert_eq!(remainder, 0);
    }

    #[test]
    fn wheel_event_routes_authenticated_persistent_regions_without_viewer_input_owner() {
        let source = include_str!("app.rs");
        let start = source.find("WindowEvent::Wheel { delta, x, y }").unwrap();
        let end = source[start..]
            .find("WindowEvent::CaptureLost")
            .map(|offset| start + offset)
            .unwrap();
        let wheel_route = &source[start..end];
        assert!(wheel_route.contains("xui15_wheel_notches_at"));
        assert!(wheel_route.contains("xui15_blocks_lower_pointer_input"));
        assert!(wheel_route.contains("scroll_authored_wheel_notches_at"));
        assert!(wheel_route.contains("authored_pointer_blocks_lower_input"));
        assert!(wheel_route.contains("!xui15_consumed"));
        assert!(wheel_route.contains("!authored_consumed"));
        assert!(wheel_route.contains("!xui15_blocks_lower_input"));
        assert!(wheel_route.contains("!authored_blocks_lower_input"));
        assert!(!wheel_route.contains("authored_input_window_active"));
    }

    #[test]
    fn xui15_root_blocks_every_lower_pointer_route() {
        let source = include_str!("app.rs");
        let start = source.find("WindowEvent::MouseMove { x, y }").unwrap();
        let end = source[start..]
            .find("WindowEvent::CaptureLost")
            .map(|offset| start + offset)
            .unwrap();
        let pointer_routes = &source[start..end];
        let mouse_move_end = pointer_routes.find("WindowEvent::MouseDown").unwrap();
        let mouse_move = &pointer_routes[..mouse_move_end];
        assert!(mouse_move.contains("let xui15_owns_pointer"));
        assert!(mouse_move.contains("xui15_has_pointer_capture"));
        assert!(mouse_move.contains("if xui15_owns_pointer"));
        assert!(mouse_move.contains("cancel_authored_pointer_capture"));
        assert!(mouse_move.contains("xui15_pointer_move"));
        assert!(mouse_move.contains("scrub_drag = false"));
        assert!(mouse_move.contains("drag = None"));
        assert!(pointer_routes.contains("!xui17_blocks_lower_input"));
        assert!(pointer_routes.contains("!xui16_blocks_lower_input"));
        assert!(pointer_routes.contains("!xui15_blocks_lower_input"));
        let xui15_press_start = pointer_routes
            .find("if xui15_blocks_lower_input {")
            .expect("XUI15-owned mouse-down branch");
        let xui15_press_end = pointer_routes[xui15_press_start..]
            .find("} else if authored_blocks_lower_input {")
            .map(|offset| xui15_press_start + offset)
            .expect("authored mouse-down fallback");
        let xui15_press = &pointer_routes[xui15_press_start..xui15_press_end];
        assert!(xui15_press.contains("cancel_authored_pointer_capture"));
        assert!(xui15_press.contains("xui15_pointer_press"));
        assert!(xui15_press.contains("scrub_drag = false"));
        assert!(xui15_press.contains("drag = None"));
        assert!(pointer_routes.contains("xui15_pointer_release"));
        assert!(pointer_routes.contains("&& !xui15_blocks_lower_input"));
    }

    #[test]
    fn xui10_presented_surface_owns_select_pan_release_and_lower_layer_blocking() {
        let source = include_str!("app.rs");
        let start = source.find("WindowEvent::MouseMove { x, y }").unwrap();
        let end = source[start..]
            .find("WindowEvent::CaptureLost")
            .map(|offset| start + offset)
            .unwrap();
        let pointer_routes = &source[start..end];
        assert!(pointer_routes.contains("xui10_pointer_move"));
        assert!(pointer_routes.contains("xui10_blocks_lower_pointer_input"));
        assert!(pointer_routes.contains("xui10_pointer_press"));
        assert!(pointer_routes.contains("xui10_pointer_release"));
        assert!(pointer_routes.contains("let xui10_had_capture"));
        assert!(pointer_routes.contains("|| xui10_had_capture"));
        assert!(pointer_routes.contains("!xui10_blocks_lower_input"));
        assert!(pointer_routes.contains("MouseButton::Middle && xui10_blocks_lower_input"));
    }

    #[test]
    fn xui16_panel_blocks_every_lower_pointer_route() {
        let source = include_str!("app.rs");
        let start = source.find("WindowEvent::MouseMove { x, y }").unwrap();
        let end = source[start..]
            .find("WindowEvent::CaptureLost")
            .map(|offset| start + offset)
            .unwrap();
        let pointer_routes = &source[start..end];
        assert!(pointer_routes.matches("occupation_owns_pointer(").count() >= 3);
        assert!(pointer_routes.contains("xui16_owns_pointer"));
        assert!(pointer_routes.contains("xui16_blocks_lower_input"));
        assert!(pointer_routes.contains("cancel_authored_pointer_capture"));
        assert!(pointer_routes.contains("xui15_pointer_release"));
        assert!(pointer_routes.contains("!xui16_blocks_lower_input"));
    }

    #[test]
    fn xui11_panel_blocks_left_middle_and_right_lower_pointer_routes() {
        let source = include_str!("app.rs");
        let start = source
            .find("WindowEvent::MouseDown { button, x, y }")
            .unwrap();
        let end = source[start..]
            .find("WindowEvent::Wheel")
            .map(|offset| start + offset)
            .unwrap();
        let pointer_routes = &source[start..end];
        let middle_right = pointer_routes
            .split_once("MouseButton::Middle | MouseButton::Right => {")
            .expect("middle/right route")
            .1;
        assert!(middle_right.contains("} else if xui11_blocks_lower_input {"));
        assert!(middle_right.contains("|| xui11_blocks_lower_input"));
        assert!(pointer_routes.contains("let xui11_owns_pointer"));
        assert!(pointer_routes.contains("|| xui11_owns_pointer"));
        let xui11_branch = middle_right
            .split_once("} else if xui11_blocks_lower_input {")
            .expect("XUI11-owned middle/right branch")
            .1
            .split_once("} else if xui14_blocks_lower_input")
            .expect("lower input fallback")
            .0;
        assert!(xui11_branch.contains("cancel_authored_pointer_capture"));
        assert!(xui11_branch.contains("xui15_pointer_release"));
        assert!(xui11_branch.contains("drag = None"));
    }

    #[test]
    fn xui21_presented_popup_is_topmost_for_every_pointer_and_keyboard_route() {
        let source = include_str!("app.rs");
        let start = source.find("WindowEvent::MouseMove { x, y }").unwrap();
        let end = source[start..]
            .find("WindowEvent::CaptureLost")
            .map(|offset| start + offset)
            .unwrap();
        let pointer = &source[start..end];
        assert!(pointer.contains("xui21_pointer_move"));
        assert!(pointer.matches("xui21_blocks_lower_pointer_input").count() >= 4);
        assert!(pointer.contains("if xui21_blocks_lower_input {"));
        assert!(pointer.contains("let xui21_owns_pointer"));
        assert!(pointer.contains("if xui21_owns_pointer"));
        assert!(pointer.contains("if !xui21_blocks_lower_input"));
        let character_start = pointer.find("let xui17_scrolled =").unwrap();
        let character_tail = &pointer[character_start..];
        let character_wheel =
            &character_tail[..character_tail.find("let xui16_blocks_lower_input").unwrap()];
        assert!(character_wheel.contains("!xui21_blocks_lower_input"));
        assert!(character_wheel.contains("!xui19_blocks_lower_input"));
        assert!(character_wheel.contains("route_character_wheel("));
        let occupation_start = pointer.find("let xui16_scrolled =").unwrap();
        let occupation_tail = &pointer[occupation_start..];
        let occupation_wheel = &occupation_tail[..occupation_tail
            .find("if !xui21_blocks_lower_input")
            .unwrap()];
        assert!(occupation_wheel.contains("!xui21_blocks_lower_input"));
        assert!(occupation_wheel.contains("!xui19_blocks_lower_input"));
        assert!(occupation_wheel.contains("route_occupation_wheel("));

        let escape_start = source.find("VK_ESCAPE => {").unwrap();
        let escape_end = source[escape_start..]
            .find("VK_RETURN | VK_SPACE")
            .map(|offset| escape_start + offset)
            .unwrap();
        assert!(source[escape_start..escape_end].contains("dismiss_xui21_focused"));

        let tab_start = source.find("VK_TAB => {").unwrap();
        let tab_end = source[tab_start..]
            .find("VK_LEFT")
            .map(|offset| tab_start + offset)
            .unwrap();
        assert!(source[tab_start..tab_end].contains("cycle_xui21_focus"));
    }

    #[test]
    fn escape_closes_xui10_then_xui17_then_xui16_then_resets_xui15_before_app_exit() {
        let source = include_str!("app.rs");
        let start = source.find("VK_ESCAPE => {").unwrap();
        let end = source[start..]
            .find("VK_RETURN | VK_SPACE")
            .map(|offset| start + offset)
            .unwrap();
        let escape = &source[start..end];
        let focus_close = escape
            .find("product_observer_ui.close_xui10")
            .expect("XUI10 close branch");
        let character_close = escape
            .find("observer_viewer_ui.character_selection =")
            .expect("XUI17 close branch");
        let occupation_close = escape
            .find("observer_viewer_ui.occupation.panel =")
            .expect("XUI16 close branch");
        assert!(focus_close < character_close);
        assert!(!escape.contains("occupation_input_region = None"));
        assert!(escape.contains("if observer_viewer_ui.trade_panel_open"));
        let trade_close = escape
            .find("observer_viewer_ui.trade_panel_open = false")
            .expect("XUI15 close branch");
        assert!(escape.contains("observer_viewer_ui.trade_panel_open = false"));
        assert!(escape.contains("CountryTradeViewerState::default()"));
        let app_close = escape
            .find("closing = true")
            .expect("application close branch");
        assert!(
            character_close < occupation_close
                && occupation_close < trade_close
                && trade_close < app_close
        );
    }

    #[test]
    fn xui17_four_surface_route_owns_pointer_wheel_tooltips_and_seek_reset() {
        let source = include_str!("app.rs");
        let start = source.find("WindowEvent::MouseMove { x, y }").unwrap();
        let end = source[start..]
            .find("WindowEvent::CaptureLost")
            .map(|offset| start + offset)
            .unwrap();
        let pointer_routes = &source[start..end];
        assert!(pointer_routes.matches("character_owns_pointer(").count() >= 3);
        assert!(pointer_routes.contains("route_character_wheel("));
        assert!(pointer_routes.contains("xui17_owns_pointer"));
        assert!(pointer_routes.contains("xui17_blocks_lower_input"));
        assert!(pointer_routes.contains("!xui17_scrolled"));
        assert!(pointer_routes.contains("!xui17_blocks_lower_input"));
        assert!(pointer_routes.contains("xui17_pointer_move(x, y)"));
        for panel in ["ArmyRoster", "NavyRoster", "Aces", "TraitTree"] {
            assert!(source.contains(panel), "missing XUI17 panel route: {panel}");
        }
        let lifecycle = &source[source
            .find("// Viewer-owned XUI17 lifecycle")
            .expect("XUI17 lifecycle branch")..];
        let lifecycle_end = lifecycle
            .find("VK_OEM_COMMA")
            .expect("next keyboard route after XUI17 lifecycle");
        let lifecycle = &lifecycle[..lifecycle_end];
        assert!(lifecycle.contains("character_semantic_state.as_ref()"));
        assert!(lifecycle.contains("no admitted Presented model"));
        assert!(!lifecycle.contains("character_selection.panel ="));
        assert!(source.matches("reset_xui17_for_seek(").count() >= 6);

        let runtime = include_str!("ui_runtime.rs");
        assert!(runtime.contains("xui17_tooltip_payloads(target)"));
        assert!(runtime.contains("xui17_payloads.as_ref()"));
        assert!(runtime.contains("\"XUI17\""));
        assert!(runtime.contains("character_semantic_state = Some(model.clone())"));
        assert!(runtime.contains("panel=Closed:semantic=admitted-presented"));
        assert!(runtime.matches("self.xui17_cursor = None").count() >= 2);
    }

    #[test]
    fn xui27_mapmode_binding_is_local_when_layout_or_authored_button_is_absent() {
        let political =
            map_viewer::ViewerMapModeSelection::Hardcoded(map_viewer::HardcodedMapMode::Political);
        assert_eq!(
            authenticated_observer_mapmode_binding(&political, None),
            game_ui::hud::MapmodeBinding {
                selected_button: 0,
                use_four_button_layout: None,
            }
        );
        assert_eq!(
            authenticated_observer_mapmode_binding(&political, Some(false)),
            game_ui::hud::MapmodeBinding {
                selected_button: 1,
                use_four_button_layout: Some(false),
            }
        );
        let raids =
            map_viewer::ViewerMapModeSelection::Hardcoded(map_viewer::HardcodedMapMode::Raids);
        assert_eq!(
            authenticated_observer_mapmode_binding(&raids, Some(true)),
            game_ui::hud::MapmodeBinding {
                selected_button: 0,
                use_four_button_layout: Some(true),
            }
        );
        let source = include_str!("app.rs");
        let start = source
            .find("function_key if (VK_F1..=VK_F11).contains(&function_key)")
            .expect("eleven-button keyboard route");
        let end = source[start..]
            .find("VK_PRIOR | VK_NEXT")
            .map(|offset| start + offset)
            .expect("mapmode cycle route");
        let keyboard_route = &source[start..end];
        assert!(!keyboard_route.contains(&["VK_F", "12"].concat()));
    }

    #[test]
    fn authenticated_static_hud_dispositions_mutate_only_viewer_owned_state() {
        let mut transport = Transport::without_dates(vec![0, 1]).expect("transport");
        let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();
        let mut viewer_map_mode = map_viewer::ViewerMapModeSelection::default();
        let mut map_mode_selection_epoch = 0;

        assert!(!apply_authenticated_viewer_disposition(
            &ObserverViewerActionDisposition::SetPlaybackSpeedStep(5),
            &mut transport,
            &mut viewer,
            &mut viewer_map_mode,
            &mut map_mode_selection_epoch,
        )
        .expect("exact one-based speed action"));
        assert_eq!(transport.speed_step(), 4);

        assert!(apply_authenticated_viewer_disposition(
            &ObserverViewerActionDisposition::SelectMapmodeButton(11),
            &mut transport,
            &mut viewer,
            &mut viewer_map_mode,
            &mut map_mode_selection_epoch,
        )
        .is_err());
        assert_eq!(viewer.mapmode.selected_button, 0);

        viewer.mapmode.use_four_button_layout = Some(true);
        assert!(!apply_authenticated_viewer_disposition(
            &ObserverViewerActionDisposition::SelectMapmodeButton(1),
            &mut transport,
            &mut viewer,
            &mut viewer_map_mode,
            &mut map_mode_selection_epoch,
        )
        .expect("exact mapmode action with admitted layout"));
        assert_eq!(viewer.mapmode.selected_button, 1);
        assert_eq!(viewer.mapmode.use_four_button_layout, Some(true));
        assert_eq!(viewer_map_mode.visible_button(), Some(1));
        assert_eq!(map_mode_selection_epoch, 0);
        assert_eq!(
            viewer.theatre_mode,
            Some(game_ui::production_renderer::ObserverTheatreMode::Land)
        );

        assert!(!apply_authenticated_viewer_disposition(
            &ObserverViewerActionDisposition::OpenLandTheatreSettings(41),
            &mut transport,
            &mut viewer,
            &mut viewer_map_mode,
            &mut map_mode_selection_epoch,
        )
        .expect("authenticated land-settings identity"));
        assert_eq!(viewer.theatre_settings_land_id, Some(41));
        assert!(!apply_authenticated_viewer_disposition(
            &ObserverViewerActionDisposition::CloseLandTheatreSettings(41),
            &mut transport,
            &mut viewer,
            &mut viewer_map_mode,
            &mut map_mode_selection_epoch,
        )
        .expect("authenticated matching close"));
        assert_eq!(viewer.theatre_settings_land_id, None);

        assert!(apply_authenticated_viewer_disposition(
            &ObserverViewerActionDisposition::SelectMapmodeButton(11),
            &mut transport,
            &mut viewer,
            &mut viewer_map_mode,
            &mut map_mode_selection_epoch,
        )
        .expect("non-theatre mapmode"));
        assert_eq!(viewer_map_mode.visible_button(), Some(11));
        assert_eq!(map_mode_selection_epoch, 1);
        assert_eq!(viewer.theatre_mode, None);
        assert!(apply_authenticated_viewer_disposition(
            &ObserverViewerActionDisposition::OpenLandTheatreSettings(41),
            &mut transport,
            &mut viewer,
            &mut viewer_map_mode,
            &mut map_mode_selection_epoch,
        )
        .is_err());
    }

    #[test]
    fn selected_country_change_clears_only_country_scoped_theatre_settings() {
        let mut viewer = game_ui::production_renderer::ObserverViewerUiState {
            selected_country: Some("GER".into()),
            trade_panel_open: true,
            theatre_mode: Some(game_ui::production_renderer::ObserverTheatreMode::Land),
            theatre_settings_land_id: Some(41),
            ..Default::default()
        };
        viewer.production_lines.panel_open = true;
        viewer.production_lines.scroll_offset_px = 77;
        viewer.production_lines.set_row_presentation(
            game_ui::production_lines::Xui12RowIdentity::ExistingLine {
                kind: game_ui::production_lines::Xui12ProductionKind::Military,
                line_id: 3003,
            },
            game_ui::production_lines::ViewerRowPresentation::Collapsed,
        );
        viewer.trade.selected_resource_index = 4;
        viewer.trade.selected_country_filter_ids = vec![17, 23];
        viewer.trade.sort = game_ui::trade_layout::CountryTradeSort::Convoys;
        viewer.trade.descending = true;
        viewer.trade.catalogue_scroll.target_fixed = 900_000;
        viewer.trade.catalogue_scroll.displayed_fixed = 700_000;
        viewer.research.panel_open = true;
        viewer.research.selected_folder = Some("industry_folder".into());
        viewer.research.pan_pixels = [31, -47];
        viewer.research.selected_technology_id = Some("basic_machine_tools".into());
        let mut designer = new_designer_viewer_state(
            "GER",
            game_ui::readonly_design_peace::DesignerViewKind::Tank,
        );
        designer.anchor_design_id = "7:70".into();
        designer.anchor_catalogue_hour = 900;
        designer.selected_design_id = Some("9:70".into());
        designer.excluded_country_filter_tags = vec!["ITA".into()];
        designer.scroll_wheel_steps = 4;
        designer.country_scroll_wheel_steps = 2;
        viewer.designer = Some(designer);
        apply_selected_country_identity(&mut viewer, Some("GER".into()));
        assert_eq!(viewer.theatre_settings_land_id, Some(41));

        apply_selected_country_identity(&mut viewer, Some("ENG".into()));
        assert_eq!(viewer.selected_country.as_deref(), Some("ENG"));
        assert_eq!(viewer.theatre_settings_land_id, None);
        assert_eq!(
            viewer.theatre_mode,
            Some(game_ui::production_renderer::ObserverTheatreMode::Land)
        );
        assert!(viewer.trade_panel_open);
        assert_eq!(
            viewer.trade,
            game_ui::trade_layout::CountryTradeViewerState::default()
        );
        assert!(viewer.production_lines.panel_open);
        assert_eq!(viewer.production_lines.scroll_offset_px, 0);
        assert!(viewer.production_lines.row_presentation.is_empty());
        assert_eq!(
            viewer.research,
            game_ui::research_compositor::ResearchViewerState::default()
        );
        let designer = viewer
            .designer
            .as_ref()
            .expect("open intent survives country switch");
        assert!(designer.anchor_design_id.is_empty());
        assert_eq!(designer.anchor_catalogue_hour, -1);
        assert_eq!(designer.anchor_catalogue_owner_tag, "ENG");
        assert_eq!(designer.anchor_catalogue_family, designer.family);
        assert_eq!(designer.selected_design_id, None);
        assert!(designer.excluded_country_filter_tags.is_empty());
        assert_eq!(designer.scroll_wheel_steps, 0);
        assert_eq!(designer.country_scroll_wheel_steps, 0);

        viewer.theatre_settings_land_id = Some(52);
        apply_selected_country_identity(&mut viewer, None);
        assert_eq!(viewer.theatre_settings_land_id, None);
        assert!(!viewer.trade_panel_open);
        assert!(!viewer.production_lines.panel_open);
        assert!(viewer.designer.is_none());
    }

    #[test]
    fn authenticated_production_lines_wheel_offset_updates_viewer_state_only_when_present() {
        let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();
        viewer.production_lines.scroll_offset_px = 12;
        apply_production_lines_scroll_offset(&mut viewer, None);
        assert_eq!(viewer.production_lines.scroll_offset_px, 12);
        apply_production_lines_scroll_offset(&mut viewer, Some(40));
        assert_eq!(viewer.production_lines.scroll_offset_px, 40);
    }

    #[test]
    fn designer_filter_changes_cannot_retain_a_row_outside_effective_membership() {
        for remaining_after_filter in [
            vec!["foreign:70".to_string()],
            vec!["our:70".to_string()],
            vec!["active:70".to_string()],
            vec!["other-country:70".to_string()],
        ] {
            let mut state = new_designer_viewer_state(
                "GER",
                game_ui::readonly_design_peace::DesignerViewKind::Tank,
            );
            state.selected_design_id = Some("selected:70".into());
            reconcile_designer_selection(&mut state, &remaining_after_filter);
            assert_eq!(state.selected_design_id, None);
        }

        let mut retained = new_designer_viewer_state(
            "GER",
            game_ui::readonly_design_peace::DesignerViewKind::Tank,
        );
        retained.selected_design_id = Some("selected:70".into());
        reconcile_designer_selection(&mut retained, &["other:70".into(), "selected:70".into()]);
        assert_eq!(retained.selected_design_id.as_deref(), Some("selected:70"));
    }

    struct StartupDateProbe {
        hours: Vec<i64>,
        date_calls: AtomicUsize,
        frame_calls: AtomicUsize,
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
            self.frame_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(None)
        }

        fn date_at(&self, hour: i64) -> Result<Option<String>> {
            self.date_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(format!("1936.1.1.{hour}")))
        }
    }

    #[test]
    fn startup_transport_binding_does_not_enumerate_dates_or_load_frames() {
        let provider = StartupDateProbe {
            hours: (0..20).collect(),
            date_calls: AtomicUsize::new(0),
            frame_calls: AtomicUsize::new(0),
        };

        let transport = transport_from_provider_index(&provider).expect("transport index");

        assert_eq!(transport.len(), 20);
        assert_eq!(provider.date_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(provider.frame_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
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
            recorder_local_settings: BTreeMap::new(),
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
    }

    #[test]
    fn foundation_product_route_executes_preflight_and_debug_bundle_stays_non_product() {
        let source = include_str!("app.rs");
        let source = &source[..source
            .find("mod product_production_d3d_tests {")
            .expect("test module marker")];
        let run_start = source.find("pub fn run(").expect("shipping window entry");
        let product_gate = run_start
            + source[run_start..]
                .find("if production_recording {")
                .expect("explicit product recording gate");
        let window_foundation = product_gate
            + source[product_gate..]
                .find("ProductMapCompositionSession::start_for_window_target(")
                .expect("windowed shared XMAP22 session caller");
        let foundation = source
            .find("ProductMapFoundationSession::start_with_assigned_stage_schedule(")
            .expect("viewer-owned scheduled foundation product caller");
        let terrain_retain = source
            .find("self.terrain.retain_frame(gfx, resolver, &prepared)?;")
            .expect("shared XMAP04 terrain handoff retention");
        let political_retain = source
            .find("self.political.retain_frame(gfx, &prepared)?;")
            .expect("shared XMAP05 political handoff retention");
        let water_retain = source
            .find("self.water.retain_frame(gfx, resolver, &prepared)?;")
            .expect("shared XMAP07 water handoff retention");
        let border_retain = water_retain
            + source[water_retain..]
                .find(".thin_borders")
                .expect("shared XMAP06 border handoff retention");
        let rivers_retain = border_retain
            + source[border_retain..]
                .find(".rivers_straits")
                .expect("shared XMAP08 rivers/straits handoff retention");
        let political_bind = source
            .find(".bind_political_dependencies(gfx, &self.political)?;")
            .expect("shared XMAP05-to-XMAP04 reflected terrain binding");
        let water_bind = source
            .find(".bind_water_dependencies(gfx, &self.water)?;")
            .expect("shared XMAP07-to-XMAP04 reflected terrain binding");
        let lighting_runtime = source
            .find("impl ProductLightingHandoffRuntime")
            .expect("XMAP09 native lighting runtime");
        let lighting_runtime_end = lighting_runtime
            + source[lighting_runtime..]
                .find("impl ProductTerrainHandoffRuntime")
                .expect("XMAP09 native lighting runtime boundary");
        assert!(
            source[lighting_runtime..lighting_runtime_end]
                .contains("execute_light_depth(gfx.context.as_raw(), &projection, &mut casters)?"),
            "XMAP09 retained runtime must execute the same-device light-depth pass"
        );
        let xmap12_carrier_prepare = foundation
            + source[foundation..]
                .find(".prepare_with_xmap12_and_xmap15_inputs(")
                .expect("XMAP12 current-carrier product preparation");
        let rail_retain = source
            .find("let rail_present = self")
            .expect("XMAP16 state-first native railroad handoff retention");
        assert!(source[rail_retain..].starts_with("let rail_present = self"));
        assert!(source[rail_retain..].contains(".retain_frame(gfx, resolver, &prepared)?"));
        let rail_runtime = source
            .find("impl ProductRailHandoffRuntime")
            .expect("XMAP16 native runtime");
        let rail_runtime_end = rail_runtime
            + source[rail_runtime..]
                .find("impl ProductTerrainHandoffRuntime")
                .expect("XMAP16 native runtime boundary");
        assert!(
            source[rail_runtime..rail_runtime_end].contains("upload_definition_textures("),
            "XMAP16 normal product retention must upload admitted definition textures"
        );
        assert!(
            source[rail_runtime..rail_runtime_end].contains("draw_target_bound(")
                && source[rail_runtime..rail_runtime_end]
                    .contains("Xmap16SupplyArrowScheduleAssignment::from_prepared_frame(frame)")
                && source[rail_runtime..rail_runtime_end]
                    .contains("Xmap16SupplyArrowPass::Normal")
                && source[rail_runtime..rail_runtime_end]
                    .contains("Xmap16SupplyArrowPass::SeeThrough"),
            "XMAP16 execution must issue the authenticated rail stage and both scheduled supply-arrow passes"
        );
        assert!(
            source.contains("resource.validate_for_frame(")
                && source.contains("handoff.receipt().political_frame_sha256")
                && source.contains("receipt.camera_frame_sha256 != lighting.camera_frame_sha256")
                && source.contains("require_current_srv_device(")
                && source.contains("require_current_buffer_device("),
            "XMAP16 adapters must reject stale replay, camera, political, and D3D-device provenance"
        );
        assert!(
            source.contains("impl map_viewer::NativeXmap16HubGuiExecutor"),
            "XMAP16 must execute hubs through the shipping shared textured-quad renderer"
        );
        assert!(
            source.contains("Xmap16HubPresentationMode::Normal"),
            "XMAP16 must own an independent mode-neutral Normal hub route"
        );
        let hub_retain = source
            .find("self.rail.retain_hub_frame(")
            .expect("XMAP16 normal hub packet retention");
        let combat_retain = source
            .find("self.combat.update(prepared.combat.clone())?;")
            .expect("XMAP14 current prepared-frame retention");
        let legacy = source
            .find("if observer_render_admitted && !production_recording")
            .unwrap();
        let production_loop = source
            .find("let mut production_map_composed = false;")
            .expect("windowed production composition loop");
        let clear_target = production_loop
            + source[production_loop..]
                .find("gfx.clear_target([0.0, 0.0, 0.0, 1.0])?;")
                .expect("product target clear");
        let hub_execute = source
            .find("let receipt = rail.execute_hubs(gfx)?")
            .expect("XMAP16 normal hub execution");
        let counter_execute = source
            .find("let counter_receipt = self.xmap13_counters.execute(")
            .expect("XMAP13 final-image counter execution");
        let bucket_execute = source
            .find("xmap10.execute_map_icon_buckets(")
            .expect("XMAP22 physical map-icon bucket execution");
        let combat_execute = source
            .find("let receipt = combat")
            .expect("XMAP14 interleaved map-icon bucket execution");
        let observer_ui = legacy
            + source[legacy..]
                .find("product_observer_ui.execute(")
                .expect("shipping observer UI execution after map composition");
        let hud = source.find("overlay.draw(&gfx").unwrap();
        assert!(product_gate < window_foundation && window_foundation < legacy);
        assert!(
            terrain_retain < political_retain
                && political_retain < water_retain
                && water_retain < border_retain
                && border_retain < rivers_retain
                && rivers_retain < political_bind
                && political_bind < water_bind,
            "the shared compositor must retain XMAP04, XMAP05, XMAP07, XMAP06, and XMAP08 before dependency binding"
        );
        assert!(foundation < xmap12_carrier_prepare && xmap12_carrier_prepare < legacy);
        assert!(
            !source[foundation..legacy].contains(".prepare(frame, &viewer, content_manifest)?"),
            "the product route must not bypass current XMAP12 carrier admission"
        );
        let rivers_runtime = source
            .find("impl ProductRiversStraitsHandoffRuntime")
            .expect("XMAP08 product runtime");
        let rivers_runtime_end = rivers_runtime
            + source[rivers_runtime..]
                .find("impl ProductThinBorderHandoffRuntime")
                .expect("XMAP08 product runtime boundary");
        assert!(source[rivers_runtime..rivers_runtime_end]
            .contains(".retained_sampler_requirements()?"));
        assert!(source[rivers_runtime..rivers_runtime_end]
            .contains(".retains_constant_buffer_owner(2, \"XMAP09\")?"));
        let water_execute = source
            .find("unsafe fn execute_complete_high_gfx_frame")
            .expect("typed XMAP07 product execution seam");
        let underwater_execute = source[water_execute..]
            .find("terrain.render_underwater_refraction(")
            .map(|offset| water_execute + offset)
            .expect("real XMAP04 underwater draw");
        let ordered_surface_loop = source[underwater_execute..]
            .find(".zip(&map_copy_receipts)")
            .map(|offset| underwater_execute + offset)
            .expect("authenticated XMAP04 map-copy iteration");
        let surface_execute = source[ordered_surface_loop..]
            .find("water.draw_surface(")
            .map(|offset| ordered_surface_loop + offset)
            .expect("real XMAP07 surface draw");
        let mark_water_present = source[surface_execute..]
            .find("self.execution = ProductWaterExecutionStatus::Present")
            .map(|offset| surface_execute + offset)
            .expect("XMAP07 Present transition");
        assert!(
            water_execute < underwater_execute
                && underwater_execute < ordered_surface_loop
                && ordered_surface_loop < surface_execute
                && surface_execute < mark_water_present,
            "XMAP07 Present must follow underwater completion and every authenticated map-copy draw"
        );
        assert!(source[water_execute..mark_water_present]
            .contains("underwater_draw_receipt: Some(&underwater)"));
        assert!(source[water_execute..mark_water_present]
            .contains("underwater_terrain_frame: Some(&prepared.terrain)"));
        let terrain_execute = source
            .find(".render_visible_terrain(")
            .expect("real XMAP04 terrain execution");
        let xmap12_execute = terrain_execute
            + source[terrain_execute..]
                .find("xmap12.execute_main_scene(")
                .expect("real XMAP12 cities/buildings/fixed-object execution");
        let rivers_execute = terrain_execute
            + source[terrain_execute..]
                .find("rivers_straits.execute_complete_frame(")
                .expect("real XMAP08 rivers/straits execution");
        let early_border_execute = terrain_execute
            + source[terrain_execute..]
                .find("thin_borders.execute_phase(")
                .expect("real XMAP06 early border execution");
        let water_frame_execute = rivers_execute
            + source[rivers_execute..]
                .find("water.execute_complete_high_gfx_frame(")
                .expect("real XMAP07 water execution");
        let rail_execute = rivers_execute
            + source[rivers_execute..]
                .find("rail.execute_rail(")
                .expect("real XMAP16 railroad execution");
        let tree_execute = rail_execute
            + source[rail_execute..]
                .find("trees.execute_main_scene(")
                .expect("real XMAP11 tree execution");
        let late_border_execute = water_frame_execute
            + source[water_frame_execute..]
                .find("thin_borders.execute_phase(")
                .expect("real XMAP06 late border execution");
        let _orders_execute = source
            .find("orders.execute(")
            .expect("real XMAP15 orders/fronts/movement execution");
        let supply_execute = late_border_execute
            + source[late_border_execute..]
                .find("rail.execute_supply(")
                .expect("real XMAP16 supply-arrow execution");
        assert!(
            terrain_execute < xmap12_execute
                && xmap12_execute < early_border_execute
                && early_border_execute < rivers_execute
                && rivers_execute < rail_execute
                && rail_execute < tree_execute
                && tree_execute < water_frame_execute
                && water_frame_execute < late_border_execute
                && late_border_execute < supply_execute,
            "the proven real order must be terrain, cities/buildings/fixed objects, early borders, rivers/straits, rails, trees, water, late borders, supply arrows"
        );
        assert!(
            source[rivers_execute..water_frame_execute].contains("completed_scene_receipt_sha256")
        );
        let postfx_execute = source[mark_water_present..]
            .find("postfx.execute(")
            .map(|offset| mark_water_present + offset)
            .expect("real XMAP10 post-FX execution");
        assert!(
            mark_water_present < postfx_execute,
            "post-FX must consume a completed XMAP07 scene"
        );
        assert!(
            source[mark_water_present..postfx_execute].contains("completed_scene_receipt_sha256")
        );
        let production_call = source
            .rfind("let receipt = composition.execute_on_target(")
            .expect("windowed XMAP22 shared-compositor caller");
        let production_present = source[production_call..]
            .find("production_map_composed = true")
            .map(|offset| production_call + offset)
            .expect("production completion transition");
        let fallback_clear = source[production_present..]
            .find("if !legacy_presented")
            .map(|offset| production_present + offset)
            .expect("non-production fallback clear");
        assert!(production_call < production_present && production_present < fallback_clear);
        assert!(source[production_call..production_present].contains("&content_admission"));
        assert!(source[production_call..production_present]
            .contains("composition.map_furniture_packet().cloned()"));
        assert!(source.contains("prepared_xmap14_for_hit_test"));
        assert!(source.contains("combat.hit_test(x as f32, y as f32)"));
        assert!(source.contains("selected_xmap14_id = Some(identity.canonical_id())"));
        assert!(source.contains("combat_selection_suffix"));
        assert!(source.contains("foundation_complete: true"));
        assert!(source.contains("execution_complete: true"));
        assert!(source.contains("final_pixels_measured: true"));
        assert!(source.contains("NativeXmap15Handoff::create_on_device("));
        assert!(source.contains("native.draw(map_viewer::NativeXmap15DrawInputs"));
        assert!(source.contains("(\"LeanTexture1\", 7u32)"));
        assert!(source.contains("(\"LeanTexture2\", 8u32)"));
        assert!(source.contains("XMAP15 prepared/runtime availability join drifted"));
        assert!(source.contains("XMAP16 prepared/runtime availability join drifted"));
        assert!(source.contains("XMAP22 complete frame did not execute every packet compositor"));
        assert!(
            political_bind < rail_retain && rail_retain < combat_retain && combat_retain < legacy,
            "the normal product route must retain XMAP16 and XMAP14 after their current providers"
        );
        assert!(
            rail_retain < hub_retain
                && hub_retain < counter_execute
                && counter_execute < bucket_execute
                && bucket_execute < combat_execute
                && combat_execute < hub_execute
                && combat_execute < clear_target
                && combat_execute < observer_ui,
            "the app must physically interleave counter -> VP/resource -> combat -> hub -> raid before fallback clear and observer UI"
        );
        assert!(
            window_foundation
                < source
                    .find("admit_static_unit_catalogs(")
                    .expect("static UI catalog admission")
        );
        assert!(
            window_foundation
                < source
                    .find("let mut window = Window::create(")
                    .expect("window creation")
        );
        assert!(
            window_foundation
                < source
                    .find("let mut gfx = Gfx::create(")
                    .expect("D3D creation")
        );
        assert!(legacy < observer_ui);
        assert!(observer_ui < hud);
        assert!(source.contains("if debug_overlay_enabled && !observer_ui_executed"));
        assert!(source.contains("pov.render_status(world_frame.as_deref())"));
        assert!(source.contains("product_observer_ui.execute("));
        assert!(!source.contains(".production_render_status()"));
        assert!(!source.contains("product_production_d3d.execute("));
        assert!(!source.contains("prepared_production_d3d"));
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
    fn water_retention_defaults_to_typed_unavailable_not_present() {
        let runtime = ProductWaterHandoffRuntime::default();
        assert!(!runtime.execution_present());
        assert_eq!(
            runtime.unavailable_reason(),
            Some("XMAP10 aggregate SceneCB0/MainScene/ShadowMap providers are not retained")
        );
    }

    #[test]
    fn pointer_arbitration_matches_exact_map_icon_bucket_painter_order() {
        let unit = map_viewer::Xmap13UnitIdentity::Division {
            type_id: 1,
            stable_id: 2,
        };
        let combat = map_viewer::Xmap14IndicatorIdentity::Land {
            combat_id: 3,
            generation: 4,
        };
        let resource = map_viewer::MapFurnitureTarget::ResourceState { state_id: 5 };
        let raid = map_viewer::MapFurnitureTarget::Raid {
            raid_identities: Vec::new(),
        };

        assert_eq!(
            select_topmost_product_map_icon_hit(
                vec![resource.clone(), raid.clone()],
                Some(&combat),
                Some(&unit),
            ),
            Some(ProductMapIconPointerHit::Furniture(raid))
        );
        assert_eq!(
            select_topmost_product_map_icon_hit(vec![resource.clone()], Some(&combat), Some(&unit),),
            Some(ProductMapIconPointerHit::Combat(combat))
        );
        assert_eq!(
            select_topmost_product_map_icon_hit(vec![resource.clone()], None, Some(&unit)),
            Some(ProductMapIconPointerHit::Furniture(resource))
        );
    }

    #[test]
    fn rail_supply_retention_defaults_to_typed_unavailable_not_present() {
        let runtime = ProductRailHandoffRuntime::default();
        assert!(!runtime.execution_complete());
        assert_eq!(
            runtime.unavailable_reason(),
            Some("XMAP16 rail/supply foundation has not been retained")
        );
        assert!(!runtime.executable_frame_present());
    }

    #[test]
    fn rivers_straits_retention_defaults_to_typed_unavailable_not_empty_present() {
        let runtime = ProductRiversStraitsHandoffRuntime::default();
        match runtime.execution {
            ProductRiversStraitsExecutionStatus::Unavailable { reason } => {
                assert!(reason.contains("have not joined"));
            }
            ProductRiversStraitsExecutionStatus::Present(_) => {
                panic!("XMAP08 retention must not fabricate an authenticated-empty execution")
            }
        }
    }

    #[test]
    fn observer_ui_callsite_has_no_recorder_presentation_inputs() {
        let source = include_str!("app.rs");
        let source = &source[..source
            .find("mod product_production_d3d_tests {")
            .expect("test module marker")];
        let observer_ui = source
            .find("let mut observer_ui_executed = false")
            .expect("observer UI product route");
        let observer_ui = &source[observer_ui..];

        assert!(observer_ui.contains("product_observer_ui.execute("));
        assert!(
            !observer_ui.contains("recorded_ui_owner("),
            "observer product country must come from viewer state"
        );
        assert!(
            !observer_ui.contains("prepared_selected_present_ui"),
            "selected-Present remains an oracle, not product input"
        );
        assert!(
            !observer_ui.contains("authenticated_ui_pass"),
            "observer product pixels must not route through a recorded UI pass"
        );
        assert!(source.contains("VK_T if pov.selected_ui_country().is_some()"));
        assert!(!source.contains("VK_T if pov.active_pov().is_some()"));
    }

    #[test]
    fn xui16_observer_semantics_are_evaluated_before_observer_composition() {
        let source = include_str!("app.rs");
        let source = &source[..source
            .find("mod product_production_d3d_tests {")
            .expect("test module marker")];
        let observer = source
            .find(".refresh(frame, pov.selected_ui_country())")
            .expect("observer-owned XUI16 refresh");
        let composition = source
            .find("product_observer_ui.execute(")
            .expect("state-first observer UI route");
        assert!(observer < composition);
        assert!(source.contains("observer_occupation_outcome"));
    }

    #[test]
    fn xui20_shipping_input_is_viewer_owned_and_blocks_lower_layers() {
        let source = include_str!("app.rs");
        let production = &source[..source
            .find("mod product_production_d3d_tests {")
            .expect("test module marker")];
        let keyboard = production
            .split_once("VK_C if shift")
            .expect("Shift+C XUI20 route")
            .1;
        assert!(keyboard.contains("cycle_xui20_panel"));
        assert!(production.contains("close_xui20("));
        assert!(production.contains("xui20_pointer_move("));
        assert!(production.contains("xui20_pointer_press("));
        assert!(production.contains("xui20_wheel_notches_at("));
        assert!(
            production
                .matches("xui20_blocks_lower_pointer_input(")
                .count()
                >= 3
        );
        assert!(production.contains("|| xui20_owns_pointer"));
        assert!(production.contains("&& !xui20_blocks_lower_input"));
    }

    #[test]
    fn xui14_shipping_input_is_viewer_owned_and_blocks_lower_layers() {
        let source = include_str!("app.rs");
        let production = &source[..source
            .find("mod product_production_d3d_tests {")
            .expect("test module marker")];
        let keyboard = production
            .split_once("VK_I if shift && pov.selected_ui_country().is_some()")
            .expect("Shift+I XUI14 route")
            .1;
        assert!(keyboard.contains("toggle_xui14"));
        assert!(keyboard.contains("cycle_xui14_surface"));
        assert!(
            !keyboard[..keyboard.find("VK_I if pov.selected_ui_country()").unwrap()]
                .contains("active_pov")
        );
        assert!(production.contains("xui14_pointer_press("));
        assert!(production.contains("xui14_wheel_notches_at("));
        assert!(
            production
                .matches("xui14_blocks_lower_pointer_input(")
                .count()
                >= 4
        );
        assert!(production.contains("|| xui14_owns_pointer"));
        assert!(production.contains("&& !xui14_blocks_lower_input"));
        assert!(production
            .lines()
            .zip(production.lines().skip(1))
            .any(|(field, call)| field.trim() == ".intelligence"
                && call.trim_start().starts_with(".reconcile_country(")));
        assert!(production.contains("viewer.intelligence.close()"));
    }
}
