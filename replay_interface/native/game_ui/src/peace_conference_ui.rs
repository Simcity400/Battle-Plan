
use crate::assets::AssetResolver;
use crate::compose::{resolve_element_rect, ComposeOptions, ComposeOutput, Compositor, Rect};
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::gui_scene_util::{pair, prefix_element_names, set_position_upper_left};
use crate::loc::Localisation;
use crate::politics_state::PoliticsTooltipPayloads;
use crate::readonly_design_peace::{
    AdmittedPeaceConference, PeaceConferenceSnapshot, PeaceParticipantRole,
};
use crate::tooltip_compositor::{
    AdmittedTooltipSource, RowAvailability, TooltipLineKind, TooltipOwner, TooltipPayload,
    TooltipRow,
};
use crate::{GameUiError, Result};
use std::collections::BTreeSet;

const PEACE_SCROLL_FIXED_SCALE: i64 = 100_000;
const PEACE_SCROLLBAR_SOURCE: &str = "interface/core.gui";

pub const PEACE_CONFERENCE_GUI: &str = "interface/peaceconferencewindow.gui";
pub const PEACE_CONFERENCE_ROOT: &str = "peaceconference_full_window";

const REQUIRED_TOP_LEVEL_TEMPLATES: [&str; 11] = [
    "aggregated_action_item",
    "available_action_item",
    "conference_winner_biddings",
    "conference_beneficiary_biddings",
    "conference_bidding_aggregated_action",
    "conference_bidding_aggregated_country",
    "conference_bidding_entry",
    "conference_bidding_quick_access_button",
    "peace_expandable_separator",
    "biddings_popup_window",
    "biddings_popup_item",
];

const REQUIRED_ACTION_WIDGETS: [&str; 9] = [
    "take_state_action",
    "liberate_action",
    "puppet_action",
    "force_government_action",
    "take_navy_action",
    "demilitarize_zone_action",
    "resource_rights_action",
    "war_reparation_action",
    "dismantle_industry_action",
];

#[derive(Clone, Debug, PartialEq)]
pub struct PeaceConferenceAuthoredTemplate {
    pub name: String,
    pub element: GuiElement,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PeaceConferenceAuthoredLayout {
    pub source_file: String,
    pub root: GuiElement,
    pub templates: Vec<PeaceConferenceAuthoredTemplate>,
    pub available_actions_scrollbar: String,
    pub available_actions_wheel_factor: Option<i32>,
    pub available_actions_smooth_scrolling: bool,
    pub history_scrollbar: String,
    pub history_wheel_factor: Option<i32>,
    pub history_smooth_scrolling: bool,
    pub contested_popup_scrollbar: String,
    pub contested_popup_wheel_factor: i32,
    pub contested_popup_smooth_scrolling: bool,
    pub available_actions_smooth_factor_fixed: i64,
    pub history_smooth_factor_fixed: i64,
    pub contested_popup_smooth_factor_fixed: i64,
}

impl PeaceConferenceAuthoredLayout {
    pub fn template(&self, name: &str) -> Option<&GuiElement> {
        self.templates
            .iter()
            .find(|template| template.name.eq_ignore_ascii_case(name))
            .map(|template| &template.element)
    }
}

fn fail(message: impl Into<String>) -> GameUiError {
    GameUiError::parse(format!("XUI08 authored layout: {}", message.into()))
}

fn last_top_level<'a>(layouts: &'a LayoutSet, name: &str) -> Option<&'a GuiElement> {
    layouts
        .documents
        .iter()
        .flat_map(|document| document.roots.iter())
        .rev()
        .find(|element| element.name.eq_ignore_ascii_case(name))
}

fn unique_descendant<'a>(root: &'a GuiElement, name: &str) -> Result<&'a GuiElement> {
    let mut matches = Vec::new();
    root.find_by_name(name, &mut matches);
    match matches.as_slice() {
        [element] => Ok(*element),
        [] => Err(fail(format!("required descendant {name:?} is absent"))),
        _ => Err(fail(format!("required descendant {name:?} is ambiguous"))),
    }
}

fn required_scalar(element: &GuiElement, field: &str) -> Result<String> {
    element
        .raw
        .first_scalar_ci(field)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| fail(format!("{} lacks {field}", element.name)))
}

fn optional_positive_scroll_factor(element: &GuiElement) -> Result<Option<i32>> {
    let Some(raw) = element.raw.first_scalar_ci("scroll_wheel_factor") else {
        return Ok(None);
    };
    let value = raw.trim().parse::<i32>().map_err(|_| {
        fail(format!(
            "{} scroll_wheel_factor is not an integer",
            element.name
        ))
    })?;
    if value <= 0 {
        return Err(fail(format!(
            "{} scroll_wheel_factor is not positive",
            element.name
        )));
    }
    Ok(Some(value))
}

fn optional_smooth_scrolling(element: &GuiElement) -> Result<bool> {
    match element
        .raw
        .first_scalar_ci("smooth_scrolling")
        .map(str::trim)
    {
        None => Ok(false),
        Some(value) if value.eq_ignore_ascii_case("yes") || value == "1" => Ok(true),
        Some(value) if value.eq_ignore_ascii_case("no") || value == "0" => Ok(false),
        Some(_) => Err(fail(format!(
            "{} smooth_scrolling is not boolean",
            element.name
        ))),
    }
}

fn referenced_scrollbar_smooth_factor(resolver: &AssetResolver, style_name: &str) -> Result<i64> {
    let mut cache = std::collections::BTreeMap::new();
    let style = crate::gui_scene_util::load_named_element(
        &mut cache,
        resolver,
        PEACE_SCROLLBAR_SOURCE,
        style_name,
    )?;
    if !style
        .element_type
        .eq_ignore_ascii_case("extendedScrollbarType")
    {
        return Err(fail(format!(
            "referenced scrollbar {style_name:?} is not an extendedScrollbarType"
        )));
    }
    let raw = required_scalar(&style, "smooth_scrolling")?;
    let value = raw.parse::<f64>().map_err(|_| {
        fail(format!(
            "referenced scrollbar {style_name:?} smooth_scrolling is not numeric"
        ))
    })?;
    if !value.is_finite() || !(0.0 < value && value <= 1.0) {
        return Err(fail(format!(
            "referenced scrollbar {style_name:?} smooth_scrolling is outside (0,1]"
        )));
    }
    let fixed = (value * PEACE_SCROLL_FIXED_SCALE as f64).round() as i64;
    (fixed > 0)
        .then_some(fixed)
        .ok_or_else(|| fail("referenced scrollbar smooth factor rounds to zero"))
}

pub fn load_peace_conference_authored_layout(
    resolver: &AssetResolver,
) -> Result<PeaceConferenceAuthoredLayout> {
    let resolved = resolver.resolve_file(PEACE_CONFERENCE_GUI)?;
    let mut layouts = LayoutSet::default();
    layouts.parse_content(&resolver.read_file(&resolved)?, &resolved.rel_path)?;
    if layouts.stats.parse_failures != 0
        || layouts.stats.recovery_count != 0
        || layouts.stats.decode_loss_count != 0
    {
        return Err(fail("winning GUI file was not parsed losslessly"));
    }

    let root = last_top_level(&layouts, PEACE_CONFERENCE_ROOT)
        .cloned()
        .ok_or_else(|| fail("controller root is absent"))?;
    if !root
        .element_type
        .eq_ignore_ascii_case("containerWindowType")
    {
        return Err(fail("controller root is not a containerWindowType"));
    }

    let available_list = unique_descendant(&root, "available_actions_list")?;
    let history_list = unique_descendant(&root, "country_biddings_list")?;
    if !available_list
        .element_type
        .eq_ignore_ascii_case("containerWindowType")
        || !history_list
            .element_type
            .eq_ignore_ascii_case("containerWindowType")
    {
        return Err(fail("dynamic insertion owner has the wrong element type"));
    }
    let available_parent = unique_descendant(&root, "available_actions")?;
    let history_parent = unique_descendant(&root, "current_and_history_actions")?;
    let available_actions_scrollbar = required_scalar(available_parent, "verticalScrollbar")?;
    let history_scrollbar = required_scalar(history_parent, "verticalScrollbar")?;
    let available_actions_wheel_factor = optional_positive_scroll_factor(available_parent)?;
    let history_wheel_factor = optional_positive_scroll_factor(history_parent)?;
    let available_actions_smooth_scrolling = optional_smooth_scrolling(available_parent)?;
    let history_smooth_scrolling = optional_smooth_scrolling(history_parent)?;

    for widget in REQUIRED_ACTION_WIDGETS {
        let element = unique_descendant(&root, widget)?;
        if !element.element_type.eq_ignore_ascii_case("buttonType") {
            return Err(fail(format!(
                "action widget {widget:?} is not a buttonType"
            )));
        }
    }

    let popup = last_top_level(&layouts, "biddings_popup_window")
        .ok_or_else(|| fail("biddings popup root is absent"))?;
    let popup_grid = unique_descendant(popup, "biddings_grid")?;
    let contested_popup_scrollbar = required_scalar(popup_grid, "verticalScrollbar")?;
    let contested_popup_wheel_factor = required_scalar(popup_grid, "scroll_wheel_factor")?
        .parse::<i32>()
        .map_err(|_| fail("biddings popup scroll_wheel_factor is not an integer"))?;
    if contested_popup_wheel_factor <= 0 {
        return Err(fail("biddings popup scroll_wheel_factor is not positive"));
    }
    let contested_popup_smooth_scrolling = match required_scalar(popup_grid, "smooth_scrolling")?
        .to_ascii_lowercase()
        .as_str()
    {
        "yes" => true,
        "no" => false,
        _ => return Err(fail("biddings popup smooth_scrolling is not yes/no")),
    };
    let available_actions_smooth_factor_fixed =
        referenced_scrollbar_smooth_factor(resolver, &available_actions_scrollbar)?;
    let history_smooth_factor_fixed =
        referenced_scrollbar_smooth_factor(resolver, &history_scrollbar)?;
    let contested_popup_smooth_factor_fixed =
        referenced_scrollbar_smooth_factor(resolver, &contested_popup_scrollbar)?;

    let mut templates = Vec::with_capacity(REQUIRED_TOP_LEVEL_TEMPLATES.len());
    for name in REQUIRED_TOP_LEVEL_TEMPLATES {
        let element = last_top_level(&layouts, name)
            .cloned()
            .ok_or_else(|| fail(format!("dynamic template {name:?} is absent")))?;
        templates.push(PeaceConferenceAuthoredTemplate {
            name: name.to_string(),
            element,
        });
    }

    Ok(PeaceConferenceAuthoredLayout {
        source_file: resolved.rel_path,
        root,
        templates,
        available_actions_scrollbar,
        available_actions_wheel_factor,
        available_actions_smooth_scrolling,
        history_scrollbar,
        history_wheel_factor,
        history_smooth_scrolling,
        contested_popup_scrollbar,
        contested_popup_wheel_factor,
        contested_popup_smooth_scrolling,
        available_actions_smooth_factor_fixed,
        history_smooth_factor_fixed,
        contested_popup_smooth_factor_fixed,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeaceConferenceHistoryTab {
    #[default]
    Victors,
    Beneficiaries,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeaceConferenceScrollRegion {
    AvailableActions,
    CurrentAndHistory,
    ContestedPopup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeaceConferenceScrollbarDrag {
    pub region: PeaceConferenceScrollRegion,
    pub grab_offset_y: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeaceConferenceViewerState {
    pub open: bool,
    pub history_tab: PeaceConferenceHistoryTab,
    pub selected_participant_id: Option<String>,
    pub selected_allocation_id: Option<String>,
    pub expanded_participant_ids: BTreeSet<String>,
    pub expanded_action_kinds: BTreeSet<String>,
    pub available_actions_scroll_px: i32,
    pub current_and_history_scroll_px: i32,
    pub contested_popup_scroll_px: i32,
    pub active_scrollbar_drag: Option<PeaceConferenceScrollbarDrag>,
    available_actions_displayed_fixed: i64,
    available_actions_target_fixed: i64,
    history_displayed_fixed: i64,
    history_target_fixed: i64,
    contested_displayed_fixed: i64,
    contested_target_fixed: i64,
    conference_id: Option<String>,
    snapshot_identity: Option<String>,
}

impl PeaceConferenceViewerState {
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    pub fn conference_id(&self) -> Option<&str> {
        self.conference_id.as_deref()
    }

    pub fn snapshot_identity(&self) -> Option<&str> {
        self.snapshot_identity.as_deref()
    }

    pub fn set_scroll(
        &mut self,
        region: PeaceConferenceScrollRegion,
        requested_px: i32,
        maximum_px: i32,
    ) -> Result<()> {
        if requested_px < 0 || maximum_px < 0 {
            return Err(fail("viewer scroll range is negative"));
        }
        let value = requested_px.min(maximum_px);
        let fixed = i64::from(value)
            .checked_mul(PEACE_SCROLL_FIXED_SCALE)
            .ok_or_else(|| fail("viewer scroll fixed-point value overflows"))?;
        match region {
            PeaceConferenceScrollRegion::AvailableActions => {
                self.available_actions_scroll_px = value;
                self.available_actions_displayed_fixed = fixed;
                self.available_actions_target_fixed = fixed;
            }
            PeaceConferenceScrollRegion::CurrentAndHistory => {
                self.current_and_history_scroll_px = value;
                self.history_displayed_fixed = fixed;
                self.history_target_fixed = fixed;
            }
            PeaceConferenceScrollRegion::ContestedPopup => {
                self.contested_popup_scroll_px = value;
                self.contested_displayed_fixed = fixed;
                self.contested_target_fixed = fixed;
            }
        }
        Ok(())
    }

    pub fn clamp_scroll(
        &mut self,
        region: PeaceConferenceScrollRegion,
        maximum_px: i32,
    ) -> Result<()> {
        if maximum_px < 0 {
            return Err(fail("viewer scroll range is negative"));
        }
        let maximum_fixed = i64::from(maximum_px)
            .checked_mul(PEACE_SCROLL_FIXED_SCALE)
            .ok_or_else(|| fail("viewer scroll maximum overflows"))?;
        let (displayed, target) = self.scroll_fixed_mut(region);
        *displayed = (*displayed).clamp(0, maximum_fixed);
        *target = (*target).clamp(0, maximum_fixed);
        self.sync_scroll_pixels(region)
    }

    pub fn select_allocation(&mut self, allocation_id: String) {
        self.selected_allocation_id = Some(allocation_id);
        self.reset_contested_popup_scroll();
    }

    pub fn clear_contested_popup(&mut self) {
        self.selected_allocation_id = None;
        self.reset_contested_popup_scroll();
    }

    fn reset_contested_popup_scroll(&mut self) {
        self.contested_popup_scroll_px = 0;
        self.contested_displayed_fixed = 0;
        self.contested_target_fixed = 0;
        if self
            .active_scrollbar_drag
            .is_some_and(|drag| drag.region == PeaceConferenceScrollRegion::ContestedPopup)
        {
            self.active_scrollbar_drag = None;
        }
    }

    pub fn wheel(
        &mut self,
        region: PeaceConferenceScrollRegion,
        signed_notches: i32,
        wheel_pixels: i32,
        maximum_px: i32,
        smooth: bool,
    ) -> Result<()> {
        if wheel_pixels <= 0 || maximum_px < 0 {
            return Err(fail("viewer wheel law is not positive"));
        }
        let maximum_fixed = i64::from(maximum_px)
            .checked_mul(PEACE_SCROLL_FIXED_SCALE)
            .ok_or_else(|| fail("viewer wheel maximum overflows"))?;
        let delta = i64::from(signed_notches)
            .checked_mul(i64::from(wheel_pixels))
            .and_then(|value| value.checked_mul(PEACE_SCROLL_FIXED_SCALE))
            .ok_or_else(|| fail("viewer wheel delta overflows"))?;
        let (displayed, target) = self.scroll_fixed_mut(region);
        *target = target
            .checked_sub(delta)
            .ok_or_else(|| fail("viewer wheel target overflows"))?
            .clamp(0, maximum_fixed);
        if !smooth {
            *displayed = *target;
        }
        self.sync_scroll_pixels(region)
    }

    pub fn advance_smooth_scroll(&mut self, input: &PeaceConferenceInputRegion) -> Result<()> {
        for region in [
            PeaceConferenceScrollRegion::AvailableActions,
            PeaceConferenceScrollRegion::CurrentAndHistory,
            PeaceConferenceScrollRegion::ContestedPopup,
        ] {
            let smooth = input.smooth(region);
            let factor = input.smooth_factor_fixed(region);
            let (displayed, target) = self.scroll_fixed_mut(region);
            *displayed = crate::scrollbar_law::extended_displayed_value(
                *displayed, *target, factor, smooth,
            )?;
            self.sync_scroll_pixels(region)?;
        }
        Ok(())
    }

    pub fn displayed_fixed(&self, region: PeaceConferenceScrollRegion) -> i64 {
        match region {
            PeaceConferenceScrollRegion::AvailableActions => self.available_actions_displayed_fixed,
            PeaceConferenceScrollRegion::CurrentAndHistory => self.history_displayed_fixed,
            PeaceConferenceScrollRegion::ContestedPopup => self.contested_displayed_fixed,
        }
    }

    pub fn apply_scrollbar_action(
        &mut self,
        input: &PeaceConferenceInputRegion,
        action: PeaceConferenceScrollbarAction,
    ) -> Result<()> {
        match action {
            PeaceConferenceScrollbarAction::Step(region, direction) => {
                let scrollbar = input
                    .scrollbar(region)
                    .ok_or_else(|| fail("scrollbar step has no authenticated geometry"))?;
                let maximum_fixed = i64::from(input.maximum(region))
                    .checked_mul(PEACE_SCROLL_FIXED_SCALE)
                    .ok_or_else(|| fail("scrollbar step maximum overflows"))?;
                let delta = i64::from(direction)
                    .checked_mul(i64::from(scrollbar.step_pixels))
                    .and_then(|value| value.checked_mul(PEACE_SCROLL_FIXED_SCALE))
                    .ok_or_else(|| fail("scrollbar step overflows"))?;
                let (displayed, target) = self.scroll_fixed_mut(region);
                *target = target
                    .checked_add(delta)
                    .ok_or_else(|| fail("scrollbar target overflows"))?
                    .clamp(0, maximum_fixed);
                if !input.smooth(region) {
                    *displayed = *target;
                }
                self.sync_scroll_pixels(region)?;
            }
            PeaceConferenceScrollbarAction::BeginThumb(region, grab_offset_y) => {
                self.active_scrollbar_drag = Some(PeaceConferenceScrollbarDrag {
                    region,
                    grab_offset_y,
                });
            }
            PeaceConferenceScrollbarAction::BeginTrack(region, pointer_y) => {
                let scrollbar = input
                    .scrollbar(region)
                    .ok_or_else(|| fail("scrollbar track has no authenticated geometry"))?;
                let grab_offset_y = scrollbar.geometry.slider.rect.h() / 2;
                self.set_scrollbar_from_pointer(input, region, pointer_y, grab_offset_y)?;
                self.active_scrollbar_drag = Some(PeaceConferenceScrollbarDrag {
                    region,
                    grab_offset_y,
                });
            }
        }
        Ok(())
    }

    pub fn drag_active_scrollbar(
        &mut self,
        input: &PeaceConferenceInputRegion,
        pointer_y: i32,
    ) -> Result<bool> {
        let Some(drag) = self.active_scrollbar_drag else {
            return Ok(false);
        };
        self.set_scrollbar_from_pointer(input, drag.region, pointer_y, drag.grab_offset_y)?;
        Ok(true)
    }

    pub fn release_active_scrollbar(&mut self) -> bool {
        self.active_scrollbar_drag.take().is_some()
    }

    fn set_scrollbar_from_pointer(
        &mut self,
        input: &PeaceConferenceInputRegion,
        region: PeaceConferenceScrollRegion,
        pointer_y: i32,
        grab_offset_y: i32,
    ) -> Result<()> {
        let scrollbar = input
            .scrollbar(region)
            .ok_or_else(|| fail("scrollbar pointer has no authenticated geometry"))?;
        let maximum_fixed = i64::from(input.maximum(region))
            .checked_mul(PEACE_SCROLL_FIXED_SCALE)
            .ok_or_else(|| fail("scrollbar pointer maximum overflows"))?;
        let desired_slider_y = pointer_y
            .checked_sub(grab_offset_y)
            .ok_or_else(|| fail("scrollbar pointer overflows"))?;
        let value = scrollbar
            .geometry
            .value_for_slider_y(desired_slider_y, maximum_fixed)?;
        let (displayed, target) = self.scroll_fixed_mut(region);
        *displayed = value;
        *target = value;
        self.sync_scroll_pixels(region)
    }

    fn scroll_fixed_mut(&mut self, region: PeaceConferenceScrollRegion) -> (&mut i64, &mut i64) {
        match region {
            PeaceConferenceScrollRegion::AvailableActions => (
                &mut self.available_actions_displayed_fixed,
                &mut self.available_actions_target_fixed,
            ),
            PeaceConferenceScrollRegion::CurrentAndHistory => (
                &mut self.history_displayed_fixed,
                &mut self.history_target_fixed,
            ),
            PeaceConferenceScrollRegion::ContestedPopup => (
                &mut self.contested_displayed_fixed,
                &mut self.contested_target_fixed,
            ),
        }
    }

    fn sync_scroll_pixels(&mut self, region: PeaceConferenceScrollRegion) -> Result<()> {
        let displayed = match region {
            PeaceConferenceScrollRegion::AvailableActions => self.available_actions_displayed_fixed,
            PeaceConferenceScrollRegion::CurrentAndHistory => self.history_displayed_fixed,
            PeaceConferenceScrollRegion::ContestedPopup => self.contested_displayed_fixed,
        };
        let pixels = i32::try_from(displayed / PEACE_SCROLL_FIXED_SCALE)
            .map_err(|_| fail("viewer displayed scroll exceeds i32"))?;
        match region {
            PeaceConferenceScrollRegion::AvailableActions => {
                self.available_actions_scroll_px = pixels
            }
            PeaceConferenceScrollRegion::CurrentAndHistory => {
                self.current_and_history_scroll_px = pixels
            }
            PeaceConferenceScrollRegion::ContestedPopup => self.contested_popup_scroll_px = pixels,
        }
        Ok(())
    }

    /// Retain local intent until an authenticated conference can reconcile it.
    /// The previous record and its scrollbar capture cannot authorize this frame.
    pub fn suspend_unavailable_frame(&mut self) {
        self.snapshot_identity = None;
        self.active_scrollbar_drag = None;
    }

    pub fn synchronize(&mut self, conference: &AdmittedPeaceConference) {
        let snapshot = conference.render_model();
        let PeaceConferenceSnapshot::Active(active) = snapshot else {
            self.clear_content_state();
            return;
        };
        let conference_changed = self.conference_id.as_deref() != Some(&active.conference_id);
        if conference_changed {
            self.clear_content_state();
            self.conference_id = Some(active.conference_id.clone());
        }
        self.snapshot_identity = Some(active.source_record_id.clone());

        let participant_ids = active
            .participants
            .iter()
            .map(|row| row.participant_id.as_str())
            .collect::<BTreeSet<_>>();
        if self
            .selected_participant_id
            .as_deref()
            .is_some_and(|id| !participant_ids.contains(id))
        {
            self.selected_participant_id = None;
        }
        self.expanded_participant_ids
            .retain(|id| participant_ids.contains(id.as_str()));

        let group_ids = active
            .allocation_groups
            .iter()
            .map(|row| row.group_id.as_str())
            .collect::<BTreeSet<_>>();
        self.expanded_action_kinds
            .retain(|id| group_ids.contains(id.as_str()));

        let contested_allocation_ids = active
            .contested_popups
            .iter()
            .map(|popup| popup.selected_allocation_id.as_str())
            .collect::<BTreeSet<_>>();
        if self
            .selected_allocation_id
            .as_deref()
            .is_some_and(|id| !contested_allocation_ids.contains(id))
        {
            self.clear_contested_popup();
        }

        if self.history_tab == PeaceConferenceHistoryTab::Beneficiaries
            && !active
                .participants
                .iter()
                .any(|row| row.role == PeaceParticipantRole::Beneficiary)
        {
            self.history_tab = PeaceConferenceHistoryTab::Victors;
        }
    }

    fn clear_content_state(&mut self) {
        self.conference_id = None;
        self.snapshot_identity = None;
        self.selected_participant_id = None;
        self.selected_allocation_id = None;
        self.expanded_participant_ids.clear();
        self.expanded_action_kinds.clear();
        self.history_tab = PeaceConferenceHistoryTab::Victors;
        self.available_actions_scroll_px = 0;
        self.current_and_history_scroll_px = 0;
        self.contested_popup_scroll_px = 0;
        self.active_scrollbar_drag = None;
        self.available_actions_displayed_fixed = 0;
        self.available_actions_target_fixed = 0;
        self.history_displayed_fixed = 0;
        self.history_target_fixed = 0;
        self.contested_displayed_fixed = 0;
        self.contested_target_fixed = 0;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeaceConferenceHitAction {
    SelectParticipant(String),
    SelectAllocation(String),
    ToggleTargetGroup(String),
    VictorsTab,
    BeneficiariesTab,
    ClosePanel,
    CloseContestedPopup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceConferenceHitTarget {
    pub rect: [i32; 4],
    pub action: PeaceConferenceHitAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceConferenceTooltipHitTarget {
    pub rect: [i32; 4],
    pub payloads: PoliticsTooltipPayloads,
}

impl PeaceConferenceTooltipHitTarget {
    pub fn contains(&self, point: (i32, i32)) -> bool {
        rect_contains(self.rect, point)
    }
}

impl PeaceConferenceHitTarget {
    pub fn contains(&self, point: (i32, i32)) -> bool {
        point.0 >= self.rect[0]
            && point.1 >= self.rect[1]
            && point.0 < self.rect[0].saturating_add(self.rect[2])
            && point.1 < self.rect[1].saturating_add(self.rect[3])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceConferenceScrollbarInput {
    pub geometry: crate::trade_layout::TradeScrollbarGeometry,
    pub step_pixels: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeaceConferenceScrollbarAction {
    Step(PeaceConferenceScrollRegion, i32),
    BeginThumb(PeaceConferenceScrollRegion, i32),
    BeginTrack(PeaceConferenceScrollRegion, i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceConferenceInputRegion {
    pub panel: [i32; 4],
    pub available_actions: [i32; 4],
    pub current_and_history: [i32; 4],
    pub contested_popup: Option<[i32; 4]>,
    pub available_max_scroll_px: i32,
    pub history_max_scroll_px: i32,
    pub contested_max_scroll_px: i32,
    pub available_wheel_pixels: Option<i32>,
    pub history_wheel_pixels: Option<i32>,
    pub contested_wheel_pixels: Option<i32>,
    pub available_smooth: bool,
    pub history_smooth: bool,
    pub contested_smooth: bool,
    pub available_smooth_factor_fixed: i64,
    pub history_smooth_factor_fixed: i64,
    pub contested_smooth_factor_fixed: i64,
    pub available_scrollbar: PeaceConferenceScrollbarInput,
    pub history_scrollbar: PeaceConferenceScrollbarInput,
    pub contested_scrollbar: Option<PeaceConferenceScrollbarInput>,
}

impl PeaceConferenceInputRegion {
    pub fn panel_contains(&self, point: (i32, i32)) -> bool {
        rect_contains(self.panel, point)
    }

    pub fn region_at(&self, point: (i32, i32)) -> Option<PeaceConferenceScrollRegion> {
        if self
            .contested_popup
            .is_some_and(|rect| rect_contains(rect, point))
        {
            Some(PeaceConferenceScrollRegion::ContestedPopup)
        } else if rect_contains(self.available_actions, point) {
            Some(PeaceConferenceScrollRegion::AvailableActions)
        } else if rect_contains(self.current_and_history, point) {
            Some(PeaceConferenceScrollRegion::CurrentAndHistory)
        } else {
            None
        }
    }

    pub fn maximum(&self, region: PeaceConferenceScrollRegion) -> i32 {
        match region {
            PeaceConferenceScrollRegion::AvailableActions => self.available_max_scroll_px,
            PeaceConferenceScrollRegion::CurrentAndHistory => self.history_max_scroll_px,
            PeaceConferenceScrollRegion::ContestedPopup => self.contested_max_scroll_px,
        }
    }

    pub fn wheel_pixels(&self, region: PeaceConferenceScrollRegion) -> Option<i32> {
        match region {
            PeaceConferenceScrollRegion::AvailableActions => self.available_wheel_pixels,
            PeaceConferenceScrollRegion::CurrentAndHistory => self.history_wheel_pixels,
            PeaceConferenceScrollRegion::ContestedPopup => self.contested_wheel_pixels,
        }
    }

    pub fn smooth(&self, region: PeaceConferenceScrollRegion) -> bool {
        match region {
            PeaceConferenceScrollRegion::AvailableActions => self.available_smooth,
            PeaceConferenceScrollRegion::CurrentAndHistory => self.history_smooth,
            PeaceConferenceScrollRegion::ContestedPopup => self.contested_smooth,
        }
    }

    pub fn smooth_factor_fixed(&self, region: PeaceConferenceScrollRegion) -> i64 {
        match region {
            PeaceConferenceScrollRegion::AvailableActions => self.available_smooth_factor_fixed,
            PeaceConferenceScrollRegion::CurrentAndHistory => self.history_smooth_factor_fixed,
            PeaceConferenceScrollRegion::ContestedPopup => self.contested_smooth_factor_fixed,
        }
    }

    fn scrollbar(
        &self,
        region: PeaceConferenceScrollRegion,
    ) -> Option<&PeaceConferenceScrollbarInput> {
        match region {
            PeaceConferenceScrollRegion::AvailableActions => Some(&self.available_scrollbar),
            PeaceConferenceScrollRegion::CurrentAndHistory => Some(&self.history_scrollbar),
            PeaceConferenceScrollRegion::ContestedPopup => self.contested_scrollbar.as_ref(),
        }
    }

    pub fn scrollbar_action_at(
        &self,
        viewer: &PeaceConferenceViewerState,
        point: (i32, i32),
    ) -> Result<Option<PeaceConferenceScrollbarAction>> {
        for region in [
            PeaceConferenceScrollRegion::ContestedPopup,
            PeaceConferenceScrollRegion::AvailableActions,
            PeaceConferenceScrollRegion::CurrentAndHistory,
        ] {
            let inside_contested_popup = region == PeaceConferenceScrollRegion::ContestedPopup
                && self
                    .contested_popup
                    .is_some_and(|popup| rect_contains(popup, point));
            let Some(scrollbar) = self.scrollbar(region) else {
                if inside_contested_popup {
                    return Ok(None);
                }
                continue;
            };
            let maximum_fixed = i64::from(self.maximum(region))
                .checked_mul(PEACE_SCROLL_FIXED_SCALE)
                .ok_or_else(|| fail("scrollbar maximum overflows"))?;
            let slider = scrollbar
                .geometry
                .slider_rect(viewer.displayed_fixed(region), maximum_fixed)?;
            if rect_contains(rect_array(scrollbar.geometry.decrease.rect), point) {
                return Ok(Some(PeaceConferenceScrollbarAction::Step(region, -1)));
            }
            if rect_contains(rect_array(scrollbar.geometry.increase.rect), point) {
                return Ok(Some(PeaceConferenceScrollbarAction::Step(region, 1)));
            }
            if maximum_fixed > 0 && rect_contains(rect_array(slider), point) {
                return Ok(Some(PeaceConferenceScrollbarAction::BeginThumb(
                    region,
                    point.1.saturating_sub(slider.y()),
                )));
            }
            if maximum_fixed > 0 && rect_contains(rect_array(scrollbar.geometry.track.rect), point)
            {
                return Ok(Some(PeaceConferenceScrollbarAction::BeginTrack(
                    region, point.1,
                )));
            }
            if inside_contested_popup {
                return Ok(None);
            }
        }
        Ok(None)
    }
}

#[derive(Clone, Debug)]
pub struct PeaceConferenceComposeOutput {
    pub output: ComposeOutput,
    pub input_region: PeaceConferenceInputRegion,
    pub hit_targets: Vec<PeaceConferenceHitTarget>,
    pub tooltip_targets: Vec<PeaceConferenceTooltipHitTarget>,
    pub materialized_participant_ids: Vec<String>,
    pub materialized_allocation_ids: Vec<String>,
    pub materialized_result_ids: Vec<String>,
}

pub fn render_peace_conference(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    localisation: &Localisation,
    exact: &replay_core::PeaceConferencePresentationExact,
    admitted: &AdmittedPeaceConference,
    viewer: &PeaceConferenceViewerState,
    width: usize,
    height: usize,
) -> Result<Option<PeaceConferenceComposeOutput>> {
    use replay_core::PeaceConferencePresentationObservationExact as Observation;
    let Observation::Active {
        current_participant_id,
        current_participant_display,
        phase_display,
        current_score,
        score_this_turn,
        remaining_turns_display,
        participants,
        allocations,
        allocation_groups,
        contested_popups,
        results,
        ..
    } = &exact.observation
    else {
        return Ok(None);
    };
    let PeaceConferenceSnapshot::Active(active) = admitted.render_model() else {
        return Err(fail("PCS3 active carrier admitted as inactive"));
    };
    if !viewer.open
        || viewer.conference_id() != Some(active.conference_id.as_str())
        || viewer.snapshot_identity() != Some(active.source_record_id.as_str())
    {
        return Err(fail(
            "viewer state is not synchronized to the active PCS3 record",
        ));
    }
    if !localisation.decode_lossy_files.is_empty() {
        return Err(fail("effective localisation contains decode loss"));
    }

    let layout = load_peace_conference_authored_layout(resolver)?;
    let mut root = layout.root.clone();
    let mut options = ComposeOptions::new(width, height).with_localisation(localisation);
    for hidden in [
        "peaceconference_select_action_window",
        "reset_container",
        "reopen_ingame_lobby_container",
        "peaceconference_buttons",
    ] {
        options.hidden_elements.insert(hidden.into());
    }
    for disabled in ["done_button", "end_turn_button"] {
        options.disabled_button_elements.insert(disabled.into());
    }
    set_text(
        &mut options,
        "current_player",
        &current_participant_display.final_text,
    );
    set_text(&mut options, "bidphase_text", &phase_display.final_text);
    set_text(&mut options, "current_score", &current_score.final_text);
    set_text(
        &mut options,
        "scorethisturn_text",
        &score_this_turn.final_text,
    );
    set_text(
        &mut options,
        "remainingturns_text",
        &remaining_turns_display.final_text,
    );
    if current_participant_id.is_none() {
        options.hidden_elements.insert("world_tension_icon".into());
    }

    let viewport = Rect::new(
        0,
        0,
        i32::try_from(width).map_err(|_| fail("viewport width exceeds i32"))?,
        i32::try_from(height).map_err(|_| fail("viewport height exceeds i32"))?,
    );
    let available_rect = descendant_rect(&root, "available_actions", viewport, &options)?;
    let history_rect = descendant_rect(&root, "current_and_history_actions", viewport, &options)?;
    let mut scrollbar_cache = std::collections::BTreeMap::new();
    let available_scrollbar_geometry = crate::trade_layout::derive_vertical_scrollbar(
        &mut scrollbar_cache,
        resolver,
        Some(registry),
        unique_descendant(&layout.root, "available_actions")?,
        available_rect,
    )?
    .ok_or_else(|| fail("available actions lost its referenced vertical scrollbar"))?;
    let history_scrollbar_geometry = crate::trade_layout::derive_vertical_scrollbar(
        &mut scrollbar_cache,
        resolver,
        Some(registry),
        unique_descendant(&layout.root, "current_and_history_actions")?,
        history_rect,
    )?
    .ok_or_else(|| fail("history lost its referenced vertical scrollbar"))?;
    let available_scroll_step = template_height(
        layout
            .template("available_action_item")
            .ok_or_else(|| fail("available_action_item disappeared"))?,
        &options,
    )?;
    let history_scroll_step = template_height(
        layout
            .template("conference_bidding_entry")
            .ok_or_else(|| fail("conference_bidding_entry disappeared"))?,
        &options,
    )?;
    let mut available_height = 0_i32;
    let mut available_ids = Vec::new();
    let available_owner = descendant_mut_exact(&mut root, "available_actions_list")?;
    let target_groups = allocation_groups
        .iter()
        .map(|group| {
            let rows = group
                .allocation_ids
                .iter()
                .map(|id| {
                    allocations
                        .iter()
                        .find(|allocation| allocation.stable_allocation_id == *id)
                        .ok_or_else(|| fail("producer allocation group member disappeared"))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((group, rows))
        })
        .collect::<Result<Vec<_>>>()?;
    for (group_ordinal, (producer_group, rows)) in target_groups.iter().enumerate() {
        let mut group = layout
            .template("aggregated_action_item")
            .cloned()
            .ok_or_else(|| fail("aggregated_action_item disappeared"))?;
        let prefix = format!("xui08_target_{group_ordinal}_");
        let header_height = descendant_local_height(&group, "aggregated_action_header", &options)?;
        set_position_upper_left(
            &mut group,
            0,
            available_height.saturating_sub(viewer.available_actions_scroll_px),
        );
        prefix_element_names(&mut group, &prefix);
        set_text(
            &mut options,
            &format!("{prefix}action_name"),
            &producer_group.target_display.final_text,
        );
        set_text(
            &mut options,
            &format!("{prefix}number_of_available_states"),
            &producer_group.available_state_count_display.final_text,
        );
        set_text(
            &mut options,
            &format!("{prefix}number_of_wargoals"),
            &producer_group.wargoal_count_display.final_text,
        );
        set_text(
            &mut options,
            &format!("{prefix}number_of_contested_states"),
            &producer_group.contested_state_count_display.final_text,
        );
        if let Some(flag) = producer_group.flag_texture_path.as_deref() {
            bind_flag(
                resolver,
                &mut options,
                &format!("{prefix}country_flag"),
                flag,
            )?;
        } else {
            options
                .hidden_elements
                .insert(format!("{prefix}country_flag"));
        }
        let expanded = viewer
            .expanded_action_kinds
            .contains(&producer_group.stable_group_id);
        if expanded {
            let grid =
                descendant_mut_exact(&mut group, &format!("{prefix}available_actions_grid"))?;
            let row_height = template_height(
                layout
                    .template("available_action_item")
                    .ok_or_else(|| fail("available_action_item disappeared"))?,
                &options,
            )?;
            for (row_ordinal, row) in rows.iter().enumerate() {
                let mut item = layout
                    .template("available_action_item")
                    .cloned()
                    .ok_or_else(|| fail("available_action_item disappeared"))?;
                let item_prefix = format!("{prefix}allocation_{row_ordinal}_");
                set_position_upper_left(
                    &mut item,
                    0,
                    i32::try_from(row_ordinal)
                        .map_err(|_| fail("available row ordinal exceeds i32"))?
                        .checked_mul(row_height)
                        .ok_or_else(|| fail("available row position overflow"))?,
                );
                prefix_element_names(&mut item, &item_prefix);
                set_text(
                    &mut options,
                    &format!("{item_prefix}score"),
                    &row.cost.final_text,
                );
                set_text(
                    &mut options,
                    &format!("{item_prefix}action_name"),
                    &row.action_display.final_text,
                );
                let actor_flag = participants
                    .iter()
                    .find(|participant| {
                        participant.stable_participant_id == row.actor_participant_id
                    })
                    .map(|participant| participant.flag_texture_path.as_str())
                    .ok_or_else(|| fail("allocation actor flag is absent"))?;
                bind_flag(
                    resolver,
                    &mut options,
                    &format!("{item_prefix}country_flag"),
                    actor_flag,
                )?;
                if row.contested_by_participant_ids.is_empty() {
                    options
                        .hidden_elements
                        .insert(format!("{item_prefix}contested_icon"));
                }
                grid.children.push(item);
                available_ids.push(row.stable_allocation_id.clone());
            }
            available_height = available_height
                .checked_add(header_height)
                .and_then(|value| {
                    value.checked_add(i32::try_from(rows.len()).ok()?.checked_mul(row_height)?)
                })
                .ok_or_else(|| fail("available list height overflow"))?;
        } else {
            available_height = available_height
                .checked_add(header_height)
                .ok_or_else(|| fail("available list height overflow"))?;
        }
        available_owner.children.push(group);
    }

    let mut history_height = 0_i32;
    let mut participant_ids = Vec::new();
    let mut history_allocation_ids = Vec::new();
    let mut result_ids = Vec::new();
    let history_owner = descendant_mut_exact(&mut root, "country_biddings_list")?;
    let visible_participants = participants
        .iter()
        .filter(|row| match viewer.history_tab {
            PeaceConferenceHistoryTab::Victors => {
                row.role == replay_core::PeaceParticipantRoleExact::Victor
            }
            PeaceConferenceHistoryTab::Beneficiaries => {
                row.role == replay_core::PeaceParticipantRoleExact::Beneficiary
            }
        })
        .collect::<Vec<_>>();
    for (participant_ordinal, participant) in visible_participants.iter().copied().enumerate() {
        let template_name = match participant.role {
            replay_core::PeaceParticipantRoleExact::Beneficiary => {
                "conference_beneficiary_biddings"
            }
            _ => "conference_winner_biddings",
        };
        let mut group = layout
            .template(template_name)
            .cloned()
            .ok_or_else(|| fail(format!("{template_name} disappeared")))?;
        let prefix = format!("xui08_participant_{participant_ordinal}_");
        let header_height =
            descendant_local_height(&group, "conference_country_biddings_header", &options)?;
        set_position_upper_left(
            &mut group,
            0,
            history_height.saturating_sub(viewer.current_and_history_scroll_px),
        );
        prefix_element_names(&mut group, &prefix);
        set_text(
            &mut options,
            &format!("{prefix}country_name"),
            &participant.country_display.final_text,
        );
        set_text(
            &mut options,
            &format!("{prefix}score"),
            &participant.score.final_text,
        );
        set_text(
            &mut options,
            &format!("{prefix}peacescore_ratio"),
            &participant.score_ratio.final_text,
        );
        bind_flag(
            resolver,
            &mut options,
            &format!("{prefix}country_flag"),
            &participant.flag_texture_path,
        )?;
        let participant_rows = participant
            .allocation_ids
            .iter()
            .map(|id| {
                allocations
                    .iter()
                    .find(|allocation| allocation.stable_allocation_id == *id)
                    .ok_or_else(|| fail("producer participant allocation member disappeared"))
            })
            .collect::<Result<Vec<_>>>()?;
        set_text(
            &mut options,
            &format!("{prefix}number_of_bids"),
            &participant.bid_count_display.final_text,
        );
        set_text(
            &mut options,
            &format!("{prefix}number_of_contested_bids"),
            &participant.contested_bid_count_display.final_text,
        );
        let expanded = viewer
            .expanded_participant_ids
            .contains(&participant.stable_participant_id);
        let row_height = template_height(
            layout
                .template("conference_bidding_entry")
                .ok_or_else(|| fail("conference_bidding_entry disappeared"))?,
            &options,
        )?;
        let mut appended_rows = 0_usize;
        if expanded {
            let grid = descendant_mut_exact(&mut group, &format!("{prefix}biddings_grid"))?;
            for (row_ordinal, row) in participant_rows.iter().enumerate() {
                let mut item = layout
                    .template("conference_bidding_entry")
                    .cloned()
                    .ok_or_else(|| fail("conference_bidding_entry disappeared"))?;
                let item_prefix = format!("{prefix}allocation_{row_ordinal}_");
                position_prefixed_history_row(&mut item, row_ordinal, row_height, &item_prefix)?;
                set_text(
                    &mut options,
                    &format!("{item_prefix}cost"),
                    &row.cost.final_text,
                );
                set_text(
                    &mut options,
                    &format!("{item_prefix}action_name"),
                    &row.action_display.final_text,
                );
                bind_flag(
                    resolver,
                    &mut options,
                    &format!("{item_prefix}country_flag"),
                    &participant.flag_texture_path,
                )?;
                if row.contested_by_participant_ids.is_empty() {
                    options
                        .hidden_elements
                        .insert(format!("{item_prefix}contested_icon"));
                }
                grid.children.push(item);
                history_allocation_ids.push(row.stable_allocation_id.clone());
            }
            appended_rows = participant_rows.len();
            if active.phase == "completed" {
                for result_id in &participant.result_ids {
                    let result = results
                        .iter()
                        .find(|row| row.stable_result_id == *result_id)
                        .ok_or_else(|| fail("producer participant result member disappeared"))?;
                    let row_ordinal = appended_rows;
                    let mut item = layout
                        .template("conference_bidding_entry")
                        .cloned()
                        .ok_or_else(|| fail("conference_bidding_entry disappeared"))?;
                    let item_prefix = format!("{prefix}result_{}_", result.source_ordinal);
                    position_prefixed_history_row(
                        &mut item,
                        row_ordinal,
                        row_height,
                        &item_prefix,
                    )?;
                    set_text(
                        &mut options,
                        &format!("{item_prefix}cost"),
                        &result.outcome.final_text,
                    );
                    set_text(
                        &mut options,
                        &format!("{item_prefix}action_name"),
                        &result.action_display.final_text,
                    );
                    bind_flag(
                        resolver,
                        &mut options,
                        &format!("{item_prefix}country_flag"),
                        &participant.flag_texture_path,
                    )?;
                    options
                        .hidden_elements
                        .insert(format!("{item_prefix}contested_icon"));
                    grid.children.push(item);
                    appended_rows += 1;
                    result_ids.push(result.stable_result_id.clone());
                }
            }
        }
        history_height = history_height
            .checked_add(header_height)
            .and_then(|value| {
                value.checked_add(i32::try_from(appended_rows).ok()?.checked_mul(row_height)?)
            })
            .ok_or_else(|| fail("history list height overflow"))?;
        history_owner.children.push(group);
        participant_ids.push(participant.stable_participant_id.clone());
    }

    let mut contested_popup_rect = None;
    let mut contested_height = 0_i32;
    let mut popup_rows = Vec::new();
    let mut popup = None;
    let mut contested_scrollbar_geometry = None;
    let mut contested_scroll_step = None;
    if let Some((selected, contested_popup)) =
        viewer
            .selected_allocation_id
            .as_deref()
            .and_then(|selected_id| {
                allocations
                    .iter()
                    .find(|row| row.stable_allocation_id == selected_id)
                    .zip(
                        contested_popups
                            .iter()
                            .find(|popup| popup.selected_allocation_id == selected_id),
                    )
            })
    {
        let mut window = layout
            .template("biddings_popup_window")
            .cloned()
            .ok_or_else(|| fail("biddings_popup_window disappeared"))?;
        descendant_mut_exact(&mut window, "close_button")?.name = "xui08_popup_close_button".into();
        for mutation in ["select_all_button", "forfeit_button", "bid_button"] {
            options.hidden_elements.insert(mutation.into());
            options.disabled_button_elements.insert(mutation.into());
        }
        set_text(&mut options, "title", &selected.target_display.final_text);
        set_text(&mut options, "refunded_score", &selected.status.final_text);
        let contested_rows = contested_popup
            .rows
            .iter()
            .map(|popup_row| {
                allocations
                    .iter()
                    .find(|allocation| allocation.stable_allocation_id == popup_row.allocation_id)
                    .ok_or_else(|| fail("producer contested popup row member disappeared"))
            })
            .collect::<Result<Vec<_>>>()?;
        set_text(
            &mut options,
            "number_of_biddings",
            &selected.popup_bid_count_display.final_text,
        );
        let row_height = template_height(
            layout
                .template("biddings_popup_item")
                .ok_or_else(|| fail("biddings_popup_item disappeared"))?,
            &options,
        )?;
        let grid = descendant_mut_exact(&mut window, "grid")?;
        grid.name = "xui08_popup_rows".into();
        for (ordinal, row) in contested_rows.iter().enumerate() {
            let mut item = layout
                .template("biddings_popup_item")
                .cloned()
                .ok_or_else(|| fail("biddings_popup_item disappeared"))?;
            let prefix = format!("xui08_popup_{ordinal}_");
            set_position_upper_left(
                &mut item,
                0,
                i32::try_from(ordinal)
                    .map_err(|_| fail("popup row ordinal exceeds i32"))?
                    .checked_mul(row_height)
                    .ok_or_else(|| fail("popup row position overflow"))?
                    .saturating_sub(viewer.contested_popup_scroll_px),
            );
            prefix_element_names(&mut item, &prefix);
            set_text(&mut options, &format!("{prefix}cost"), &row.bid.final_text);
            set_text(
                &mut options,
                &format!("{prefix}action_name"),
                &row.action_display.final_text,
            );
            set_text(
                &mut options,
                &format!("{prefix}underlying_resource_name"),
                &row.target_display.final_text,
            );
            let actor_flag = participants
                .iter()
                .find(|participant| participant.stable_participant_id == row.actor_participant_id)
                .map(|participant| participant.flag_texture_path.as_str())
                .ok_or_else(|| fail("contested allocation actor flag is absent"))?;
            bind_flag(
                resolver,
                &mut options,
                &format!("{prefix}taker_flag"),
                actor_flag,
            )?;
            options
                .hidden_elements
                .insert(format!("{prefix}negotiator_flags"));
            grid.children.push(item);
            popup_rows.push(*row);
        }
        contested_height = i32::try_from(contested_rows.len())
            .map_err(|_| fail("popup row count exceeds i32"))?
            .checked_mul(row_height)
            .ok_or_else(|| fail("popup height overflow"))?;
        options.hidden_elements.insert("no_entry_info".into());
        contested_popup_rect = Some(descendant_rect(
            &window,
            "biddings_grid",
            viewport,
            &options,
        )?);
        let popup_rect =
            contested_popup_rect.ok_or_else(|| fail("contested popup clip disappeared"))?;
        contested_scrollbar_geometry = crate::trade_layout::derive_vertical_scrollbar(
            &mut scrollbar_cache,
            resolver,
            Some(registry),
            unique_descendant(&window, "biddings_grid")?,
            popup_rect,
        )?;
        if contested_scrollbar_geometry.is_none() {
            return Err(fail(
                "contested popup lost its referenced vertical scrollbar",
            ));
        }
        contested_scroll_step = Some(row_height);
        popup = Some(window);
    }

    let available_max_scroll_px = available_height.saturating_sub(available_rect.h()).max(0);
    let history_max_scroll_px = history_height.saturating_sub(history_rect.h()).max(0);
    let contested_max_scroll_px = contested_popup_rect
        .map(|rect| contested_height.saturating_sub(rect.h()).max(0))
        .unwrap_or_default();
    let available_scroll_px = viewer
        .available_actions_scroll_px
        .clamp(0, available_max_scroll_px);
    let history_scroll_px = viewer
        .current_and_history_scroll_px
        .clamp(0, history_max_scroll_px);
    let contested_scroll_px = viewer
        .contested_popup_scroll_px
        .clamp(0, contested_max_scroll_px);

    {
        let owner = descendant_mut_exact(&mut root, "available_actions_list")?;
        let mut y = 0_i32;
        for (ordinal, ((producer_group, rows), group)) in target_groups
            .iter()
            .zip(owner.children.iter_mut())
            .enumerate()
        {
            set_position_upper_left(group, 0, y.saturating_sub(available_scroll_px));
            let prefix = format!("xui08_target_{ordinal}_");
            let header_height = descendant_local_height(
                group,
                &format!("{prefix}aggregated_action_header"),
                &options,
            )?;
            y = y
                .checked_add(header_height)
                .ok_or_else(|| fail("available list height overflow"))?;
            if viewer
                .expanded_action_kinds
                .contains(&producer_group.stable_group_id)
            {
                let row_height = template_height(
                    layout
                        .template("available_action_item")
                        .ok_or_else(|| fail("available_action_item disappeared"))?,
                    &options,
                )?;
                y = y
                    .checked_add(
                        i32::try_from(rows.len())
                            .map_err(|_| fail("available row count exceeds i32"))?
                            .checked_mul(row_height)
                            .ok_or_else(|| fail("available list height overflow"))?,
                    )
                    .ok_or_else(|| fail("available list height overflow"))?;
            }
        }
    }
    {
        let owner = descendant_mut_exact(&mut root, "country_biddings_list")?;
        let mut y = 0_i32;
        for (ordinal, (participant, group)) in visible_participants
            .iter()
            .copied()
            .zip(owner.children.iter_mut())
            .enumerate()
        {
            set_position_upper_left(group, 0, y.saturating_sub(history_scroll_px));
            let prefix = format!("xui08_participant_{ordinal}_");
            let header_height = descendant_local_height(
                group,
                &format!("{prefix}conference_country_biddings_header"),
                &options,
            )?;
            y = y
                .checked_add(header_height)
                .ok_or_else(|| fail("history list height overflow"))?;
            if viewer
                .expanded_participant_ids
                .contains(&participant.stable_participant_id)
            {
                let row_height = template_height(
                    layout
                        .template("conference_bidding_entry")
                        .ok_or_else(|| fail("conference_bidding_entry disappeared"))?,
                    &options,
                )?;
                let allocation_count = participant.allocation_ids.len();
                let result_count = if active.phase == "completed" {
                    participant.result_ids.len()
                } else {
                    0
                };
                y = y
                    .checked_add(
                        i32::try_from(allocation_count.saturating_add(result_count))
                            .map_err(|_| fail("history row count exceeds i32"))?
                            .checked_mul(row_height)
                            .ok_or_else(|| fail("history list height overflow"))?,
                    )
                    .ok_or_else(|| fail("history list height overflow"))?;
            }
        }
    }
    if let Some(window) = popup.as_mut() {
        let grid = descendant_mut_exact(window, "xui08_popup_rows")?;
        let row_height = template_height(
            layout
                .template("biddings_popup_item")
                .ok_or_else(|| fail("biddings_popup_item disappeared"))?,
            &options,
        )?;
        for (ordinal, item) in grid.children.iter_mut().enumerate() {
            set_position_upper_left(
                item,
                0,
                i32::try_from(ordinal)
                    .map_err(|_| fail("popup row ordinal exceeds i32"))?
                    .checked_mul(row_height)
                    .ok_or_else(|| fail("popup row position overflow"))?
                    .saturating_sub(contested_scroll_px),
            );
        }
    }

    let mut wrapper = wrapper_root(width, height, root)?;
    if let Some(popup) = popup {
        wrapper.children.push(popup);
    }
    let mut output = Compositor::new(resolver, registry).compose(&wrapper, &options);
    let mut static_options = options.clone();
    static_options
        .hidden_elements
        .insert("available_actions_list".into());
    static_options
        .hidden_elements
        .insert("country_biddings_list".into());
    static_options
        .hidden_elements
        .insert("xui08_popup_rows".into());
    let mut clipped = Compositor::new(resolver, registry).compose(&wrapper, &static_options);
    copy_rect(&output.image, &mut clipped.image, available_rect);
    copy_rect(&output.image, &mut clipped.image, history_rect);
    if let Some(rect) = contested_popup_rect {
        copy_rect(&output.image, &mut clipped.image, rect);
    }
    output.image = clipped.image;

    crate::trade_compositor::draw_trade_scrollbar(
        resolver,
        registry,
        &mut output,
        &available_scrollbar_geometry,
        viewer
            .displayed_fixed(PeaceConferenceScrollRegion::AvailableActions)
            .clamp(
                0,
                i64::from(available_max_scroll_px) * PEACE_SCROLL_FIXED_SCALE,
            ),
        i64::from(available_max_scroll_px) * PEACE_SCROLL_FIXED_SCALE,
    )?;
    crate::trade_compositor::draw_trade_scrollbar(
        resolver,
        registry,
        &mut output,
        &history_scrollbar_geometry,
        viewer
            .displayed_fixed(PeaceConferenceScrollRegion::CurrentAndHistory)
            .clamp(
                0,
                i64::from(history_max_scroll_px) * PEACE_SCROLL_FIXED_SCALE,
            ),
        i64::from(history_max_scroll_px) * PEACE_SCROLL_FIXED_SCALE,
    )?;
    if let Some(scrollbar) = contested_scrollbar_geometry.as_ref() {
        crate::trade_compositor::draw_trade_scrollbar(
            resolver,
            registry,
            &mut output,
            scrollbar,
            viewer
                .displayed_fixed(PeaceConferenceScrollRegion::ContestedPopup)
                .clamp(
                    0,
                    i64::from(contested_max_scroll_px) * PEACE_SCROLL_FIXED_SCALE,
                ),
            i64::from(contested_max_scroll_px) * PEACE_SCROLL_FIXED_SCALE,
        )?;
    }

    let contested_allocation_ids = contested_popups
        .iter()
        .map(|popup| popup.selected_allocation_id.as_str())
        .collect::<BTreeSet<_>>();
    let hit_targets = hit_targets_from_debug(
        &output,
        &visible_participants,
        allocations,
        &target_groups,
        &contested_allocation_ids,
        available_rect,
        history_rect,
        contested_popup_rect,
    );
    let tooltip_targets = tooltip_targets_from_debug(
        &output,
        exact,
        &visible_participants,
        allocations,
        results,
        &target_groups,
        available_rect,
        history_rect,
        contested_popup_rect,
        &popup_rows,
        &active.conference_id,
        &layout.source_file,
        &crate::tooltip_compositor::authored_tooltip_shell_name(resolver)?,
    )?;
    Ok(Some(PeaceConferenceComposeOutput {
        output,
        input_region: PeaceConferenceInputRegion {
            panel: [0, 0, viewport.w(), viewport.h()],
            available_actions: rect_array(available_rect),
            current_and_history: rect_array(history_rect),
            contested_popup: contested_popup_rect.map(rect_array),
            available_max_scroll_px,
            history_max_scroll_px,
            contested_max_scroll_px,
            available_wheel_pixels: layout.available_actions_wheel_factor,
            history_wheel_pixels: layout.history_wheel_factor,
            contested_wheel_pixels: Some(layout.contested_popup_wheel_factor),
            available_smooth: layout.available_actions_smooth_scrolling,
            history_smooth: layout.history_smooth_scrolling,
            contested_smooth: layout.contested_popup_smooth_scrolling,
            available_smooth_factor_fixed: layout.available_actions_smooth_factor_fixed,
            history_smooth_factor_fixed: layout.history_smooth_factor_fixed,
            contested_smooth_factor_fixed: layout.contested_popup_smooth_factor_fixed,
            available_scrollbar: PeaceConferenceScrollbarInput {
                geometry: available_scrollbar_geometry,
                step_pixels: available_scroll_step,
            },
            history_scrollbar: PeaceConferenceScrollbarInput {
                geometry: history_scrollbar_geometry,
                step_pixels: history_scroll_step,
            },
            contested_scrollbar: contested_scrollbar_geometry.map(|geometry| {
                PeaceConferenceScrollbarInput {
                    geometry,
                    step_pixels: contested_scroll_step
                        .expect("contested scrollbar always has its row step"),
                }
            }),
        },
        hit_targets,
        tooltip_targets,
        materialized_participant_ids: participant_ids,
        materialized_allocation_ids: available_ids
            .into_iter()
            .chain(history_allocation_ids)
            .collect(),
        materialized_result_ids: result_ids,
    }))
}

fn bind_flag(
    resolver: &AssetResolver,
    options: &mut ComposeOptions,
    element_name: &str,
    texture_path: &str,
) -> Result<()> {
    let decoded = resolver.decode_texture(texture_path).map_err(|error| {
        fail(format!(
            "effective flag texture {texture_path:?} did not decode: {error}"
        ))
    })?;
    options
        .sprite_overrides
        .insert(element_name.to_string(), decoded.image);
    Ok(())
}

fn position_prefixed_history_row(
    item: &mut GuiElement,
    ordinal: usize,
    row_height: i32,
    prefix: &str,
) -> Result<()> {
    let y = i32::try_from(ordinal)
        .map_err(|_| fail("history row ordinal exceeds i32"))?
        .checked_mul(row_height)
        .ok_or_else(|| fail("history row position overflow"))?;
    set_position_upper_left(item, 0, y);
    prefix_element_names(item, prefix);
    Ok(())
}

fn template_height(element: &GuiElement, options: &ComposeOptions) -> Result<i32> {
    let rect = resolve_element_rect(element, Rect::new(0, 0, 32_767, 32_767), (0, 0), options);
    (rect.h() > 0).then_some(rect.h()).ok_or_else(|| {
        fail(format!(
            "template {:?} has no authored height",
            element.name
        ))
    })
}

fn descendant_local_height(root: &GuiElement, name: &str, options: &ComposeOptions) -> Result<i32> {
    let element = descendant_exact(root, name)?;
    template_height(element, options)
}

fn descendant_exact<'a>(root: &'a GuiElement, name: &str) -> Result<&'a GuiElement> {
    let mut hits = Vec::new();
    root.find_by_name(name, &mut hits);
    match hits.as_slice() {
        [value] => Ok(*value),
        _ => Err(fail(format!(
            "expected one descendant {name:?}, found {}",
            hits.len()
        ))),
    }
}

fn descendant_mut_exact<'a>(root: &'a mut GuiElement, name: &str) -> Result<&'a mut GuiElement> {
    fn count(root: &GuiElement, name: &str) -> usize {
        usize::from(root.name == name)
            + root
                .children
                .iter()
                .map(|child| count(child, name))
                .sum::<usize>()
    }
    fn find<'a>(root: &'a mut GuiElement, name: &str) -> Option<&'a mut GuiElement> {
        if root.name == name {
            Some(root)
        } else {
            root.children.iter_mut().find_map(|child| find(child, name))
        }
    }
    let found = count(root, name);
    if found != 1 {
        return Err(fail(format!(
            "expected one mutable descendant {name:?}, found {found}"
        )));
    }
    find(root, name).ok_or_else(|| fail(format!("descendant {name:?} disappeared")))
}

fn descendant_rect(
    root: &GuiElement,
    name: &str,
    viewport: Rect,
    options: &ComposeOptions,
) -> Result<Rect> {
    fn walk(
        root: &GuiElement,
        name: &str,
        parent: Rect,
        options: &ComposeOptions,
        hits: &mut Vec<Rect>,
    ) {
        let rect = resolve_element_rect(root, parent, (0, 0), options);
        if root.name == name {
            hits.push(rect);
        }
        for child in &root.children {
            walk(child, name, rect, options, hits);
        }
    }
    let mut hits = Vec::new();
    walk(root, name, viewport, options, &mut hits);
    match hits.as_slice() {
        [rect] => Ok(*rect),
        _ => Err(fail(format!(
            "expected one composed rect for {name:?}, found {}",
            hits.len()
        ))),
    }
}

fn wrapper_root(width: usize, height: usize, child: GuiElement) -> Result<GuiElement> {
    Ok(GuiElement {
        element_type: "containerWindowType".into(),
        name: "xui08_peace_root".into(),
        position: None,
        size: Some(pair(
            i32::try_from(width).map_err(|_| fail("viewport width overflow"))?,
            i32::try_from(height).map_err(|_| fail("viewport height overflow"))?,
        )),
        orientation: Some("UPPER_LEFT".into()),
        sprite_refs: Vec::new(),
        font: None,
        text: None,
        format: None,
        children: vec![child],
        raw: crate::paradox_text::Value::Object(Vec::new()),
    })
}

fn set_text(options: &mut ComposeOptions, name: &str, text: impl Into<String>) {
    options.text_overrides.insert(name.into(), text.into());
}

fn rect_array(rect: Rect) -> [i32; 4] {
    [rect.x(), rect.y(), rect.w(), rect.h()]
}

fn rect_contains(rect: [i32; 4], point: (i32, i32)) -> bool {
    point.0 >= rect[0]
        && point.1 >= rect[1]
        && point.0 < rect[0].saturating_add(rect[2])
        && point.1 < rect[1].saturating_add(rect[3])
}

fn copy_rect(source: &crate::dds::RgbaImage, destination: &mut crate::dds::RgbaImage, rect: Rect) {
    let x0 = rect.x().max(0) as usize;
    let y0 = rect.y().max(0) as usize;
    let x1 = rect.x().saturating_add(rect.w()).max(0) as usize;
    let y1 = rect.y().saturating_add(rect.h()).max(0) as usize;
    let x1 = x1.min(source.width).min(destination.width);
    let y1 = y1.min(source.height).min(destination.height);
    for y in y0.min(y1)..y1 {
        let start = (y * source.width + x0.min(x1)) * 4;
        let end = (y * source.width + x1) * 4;
        let destination_start = (y * destination.width + x0.min(x1)) * 4;
        let destination_end = destination_start + end.saturating_sub(start);
        destination.pixels[destination_start..destination_end]
            .copy_from_slice(&source.pixels[start..end]);
    }
}

fn hit_targets_from_debug(
    output: &ComposeOutput,
    visible_participants: &[&replay_core::PeaceParticipantPresentationExact],
    allocations: &[replay_core::PeaceAllocationPresentationExact],
    target_groups: &[(
        &replay_core::PeaceAllocationGroupPresentationExact,
        Vec<&replay_core::PeaceAllocationPresentationExact>,
    )],
    contested_allocation_ids: &BTreeSet<&str>,
    available_clip: Rect,
    history_clip: Rect,
    popup_clip: Option<Rect>,
) -> Vec<PeaceConferenceHitTarget> {
    let mut targets = Vec::new();
    for debug in &output.debug_rects {
        let resolved = if debug.name == "victors_tab" {
            Some((PeaceConferenceHitAction::VictorsTab, None, true))
        } else if debug.name == "beneficiaries_tab" {
            Some((PeaceConferenceHitAction::BeneficiariesTab, None, true))
        } else if debug.name == "close_button" {
            Some((PeaceConferenceHitAction::ClosePanel, None, true))
        } else if debug.name == "xui08_popup_close_button" {
            popup_clip.map(|clip| {
                (
                    PeaceConferenceHitAction::CloseContestedPopup,
                    Some(clip),
                    false,
                )
            })
        } else if let Some(index) = prefixed_index(&debug.name, "xui08_participant_", "_expand") {
            visible_participants.get(index).map(|row| {
                (
                    PeaceConferenceHitAction::SelectParticipant(row.stable_participant_id.clone()),
                    Some(history_clip),
                    true,
                )
            })
        } else if let Some(index) = prefixed_index(&debug.name, "xui08_target_", "_expand") {
            target_groups.get(index).map(|(group, _)| {
                (
                    PeaceConferenceHitAction::ToggleTargetGroup(group.stable_group_id.clone()),
                    Some(available_clip),
                    true,
                )
            })
        } else if let Some((group_index, allocation_index)) =
            nested_prefixed_indices(&debug.name, "xui08_target_", "_allocation_", "_Background")
        {
            target_groups
                .get(group_index)
                .and_then(|(_, rows)| rows.get(allocation_index))
                .filter(|row| {
                    contested_allocation_ids.contains(row.stable_allocation_id.as_str())
                })
                .map(|row| {
                    (
                        PeaceConferenceHitAction::SelectAllocation(
                            row.stable_allocation_id.clone(),
                        ),
                        Some(available_clip),
                        true,
                    )
                })
        } else if let Some((participant_index, allocation_index)) = nested_prefixed_indices(
            &debug.name,
            "xui08_participant_",
            "_allocation_",
            "_Background",
        ) {
            visible_participants
                .get(participant_index)
                .and_then(|participant| participant.allocation_ids.get(allocation_index))
                .and_then(|id| {
                    allocations
                        .iter()
                        .find(|allocation| allocation.stable_allocation_id == *id)
                })
                .filter(|row| {
                    contested_allocation_ids.contains(row.stable_allocation_id.as_str())
                })
                .map(|row| {
                    (
                        PeaceConferenceHitAction::SelectAllocation(
                            row.stable_allocation_id.clone(),
                        ),
                        Some(history_clip),
                        true,
                    )
                })
        } else {
            None
        };
        let Some((action, clip, occluded_by_popup)) = resolved else {
            continue;
        };
        let debug_rect = [debug.x, debug.y, debug.w, debug.h];
        let Some(clipped) = clip
            .map(rect_array)
            .map_or(Some(debug_rect), |clip| intersect_arrays(debug_rect, clip))
        else {
            continue;
        };
        let pieces = if occluded_by_popup {
            popup_clip
                .map(rect_array)
                .map_or_else(|| vec![clipped], |popup| subtract_rect(clipped, popup))
        } else {
            vec![clipped]
        };
        for rect in pieces {
            targets.push(PeaceConferenceHitTarget {
                rect,
                action: action.clone(),
            });
        }
    }
    targets
}

fn participant_result_by_source_ordinal<'a>(
    participant: &replay_core::PeaceParticipantPresentationExact,
    results: &'a [replay_core::PeaceResultPresentationExact],
    source_ordinal: usize,
) -> Option<&'a replay_core::PeaceResultPresentationExact> {
    results.iter().find(|row| {
        usize::try_from(row.source_ordinal).ok() == Some(source_ordinal)
            && participant
                .result_ids
                .iter()
                .any(|result_id| result_id == &row.stable_result_id)
    })
}

#[allow(clippy::too_many_arguments)]
fn tooltip_targets_from_debug(
    output: &ComposeOutput,
    exact: &replay_core::PeaceConferencePresentationExact,
    visible_participants: &[&replay_core::PeaceParticipantPresentationExact],
    allocations: &[replay_core::PeaceAllocationPresentationExact],
    results: &[replay_core::PeaceResultPresentationExact],
    target_groups: &[(
        &replay_core::PeaceAllocationGroupPresentationExact,
        Vec<&replay_core::PeaceAllocationPresentationExact>,
    )],
    available_clip: Rect,
    history_clip: Rect,
    popup_clip: Option<Rect>,
    popup_rows: &[&replay_core::PeaceAllocationPresentationExact],
    conference_id: &str,
    source_file: &str,
    tooltip_root_name: &str,
) -> Result<Vec<PeaceConferenceTooltipHitTarget>> {
    let mut targets = Vec::new();
    for debug in &output.debug_rects {
        let debug_rect = [debug.x, debug.y, debug.w, debug.h];
        let resolved = if let Some(index) =
            prefixed_index(&debug.name, "xui08_participant_", "_Background")
        {
            visible_participants.get(index).map(|row| {
                (
                    row.stable_participant_id.as_str(),
                    &row.tooltip,
                    rect_array(history_clip),
                    vec![0, row.source_ordinal],
                    true,
                )
            })
        } else if let Some((group_index, allocation_index)) =
            nested_prefixed_indices(&debug.name, "xui08_target_", "_allocation_", "_Background")
        {
            target_groups
                .get(group_index)
                .and_then(|(_, rows)| rows.get(allocation_index))
                .map(|row| {
                    (
                        row.stable_allocation_id.as_str(),
                        &row.tooltip,
                        rect_array(available_clip),
                        vec![1, row.source_ordinal],
                        true,
                    )
                })
        } else if let Some((participant_index, allocation_index)) = nested_prefixed_indices(
            &debug.name,
            "xui08_participant_",
            "_allocation_",
            "_Background",
        ) {
            visible_participants
                .get(participant_index)
                .and_then(|participant| {
                    participant
                        .allocation_ids
                        .get(allocation_index)
                        .and_then(|allocation_id| {
                            allocations
                                .iter()
                                .find(|row| row.stable_allocation_id == *allocation_id)
                                .map(|row| {
                                    (
                                        row.stable_allocation_id.as_str(),
                                        &row.tooltip,
                                        rect_array(history_clip),
                                        vec![2, row.source_ordinal],
                                        true,
                                    )
                                })
                        })
                })
        } else if let Some((participant_index, result_ordinal)) =
            nested_prefixed_indices(&debug.name, "xui08_participant_", "_result_", "_Background")
        {
            visible_participants
                .get(participant_index)
                .and_then(|participant| {
                    participant_result_by_source_ordinal(participant, results, result_ordinal).map(
                        |row| {
                            (
                                row.stable_result_id.as_str(),
                                &row.tooltip,
                                rect_array(history_clip),
                                vec![3, row.source_ordinal],
                                true,
                            )
                        },
                    )
                })
        } else if let Some(index) = prefixed_index(&debug.name, "xui08_popup_", "_Background") {
            popup_rows.get(index).and_then(|row| {
                popup_clip.map(|clip| {
                    (
                        row.stable_allocation_id.as_str(),
                        &row.tooltip,
                        rect_array(clip),
                        vec![4, row.source_ordinal],
                        false,
                    )
                })
            })
        } else {
            None
        };
        let Some((stable_id, tooltip, clip, element_path, occluded_by_popup)) = resolved else {
            continue;
        };
        let Some(clipped) = intersect_arrays(debug_rect, clip) else {
            continue;
        };
        let pieces = if occluded_by_popup {
            popup_clip
                .map(rect_array)
                .map_or_else(|| vec![clipped], |popup| subtract_rect(clipped, popup))
        } else {
            vec![clipped]
        };
        let payloads = peace_tooltip_payloads(
            tooltip,
            exact,
            stable_id,
            conference_id,
            source_file,
            element_path,
            tooltip_root_name,
        )?;
        targets.extend(
            pieces
                .into_iter()
                .map(|rect| PeaceConferenceTooltipHitTarget {
                    rect,
                    payloads: payloads.clone(),
                }),
        );
    }
    Ok(targets)
}

fn peace_tooltip_payloads(
    tooltip: &replay_core::PeaceTooltipExact,
    exact: &replay_core::PeaceConferencePresentationExact,
    stable_id: &str,
    conference_id: &str,
    source_file: &str,
    element_path: Vec<u32>,
    tooltip_root_name: &str,
) -> Result<PoliticsTooltipPayloads> {
    let owner = TooltipOwner {
        source_file: source_file.into(),
        root_window: PEACE_CONFERENCE_ROOT.into(),
        element_path,
        engine_instance_token: format!("xui08:{conference_id}:{stable_id}"),
    };
    let row = |ordinal: u32, field: &str, text: &replay_core::PeaceFinalTextExact| TooltipRow {
        source_ordinal: ordinal,
        line_kind: TooltipLineKind::Scripted,
        source_identity: format!("{stable_id}/{field}:{}", text.authored_identity),
        evaluation_context: format!("xui08-record={}", exact.record_sha256),
        availability: if text.final_text.is_empty() {
            RowAvailability::MeasuredEmpty
        } else {
            RowAvailability::Resolved
        },
        text: text.final_text.clone(),
        admitted_sources: vec![AdmittedTooltipSource::PacketSemantic {
            packet_id: "XUI08".into(),
            producer_identity: format!("{}:{stable_id}", exact.record_sha256),
            field: field.into(),
            manifest_sha256: exact.producer_manifest_sha256.clone(),
        }],
    };
    let primary = TooltipPayload {
        owner: owner.clone(),
        tooltip_root_name: tooltip_root_name.into(),
        rows: vec![row(0, "tooltip.primary", &tooltip.primary)],
    };
    primary.validate()?;
    let delayed = tooltip
        .delayed
        .as_ref()
        .map(|delayed| {
            let payload = TooltipPayload {
                owner,
                tooltip_root_name: tooltip_root_name.into(),
                rows: vec![
                    row(0, "tooltip.primary", &tooltip.primary),
                    row(1, "tooltip.delayed", delayed),
                ],
            };
            payload.validate().map(|_| payload)
        })
        .transpose()?;
    Ok(PoliticsTooltipPayloads { primary, delayed })
}

fn intersect_arrays(left: [i32; 4], right: [i32; 4]) -> Option<[i32; 4]> {
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

fn subtract_rect(rect: [i32; 4], occluder: [i32; 4]) -> Vec<[i32; 4]> {
    let Some(overlap) = intersect_arrays(rect, occluder) else {
        return vec![rect];
    };
    let rect_right = rect[0].saturating_add(rect[2]);
    let rect_bottom = rect[1].saturating_add(rect[3]);
    let overlap_right = overlap[0].saturating_add(overlap[2]);
    let overlap_bottom = overlap[1].saturating_add(overlap[3]);
    let mut pieces = Vec::with_capacity(4);
    if overlap[1] > rect[1] {
        pieces.push([rect[0], rect[1], rect[2], overlap[1] - rect[1]]);
    }
    if overlap_bottom < rect_bottom {
        pieces.push([
            rect[0],
            overlap_bottom,
            rect[2],
            rect_bottom - overlap_bottom,
        ]);
    }
    if overlap[0] > rect[0] {
        pieces.push([rect[0], overlap[1], overlap[0] - rect[0], overlap[3]]);
    }
    if overlap_right < rect_right {
        pieces.push([
            overlap_right,
            overlap[1],
            rect_right - overlap_right,
            overlap[3],
        ]);
    }
    pieces
}

fn prefixed_index(name: &str, prefix: &str, suffix: &str) -> Option<usize> {
    name.strip_prefix(prefix)?
        .strip_suffix(suffix)?
        .parse()
        .ok()
}

fn nested_prefixed_indices(
    name: &str,
    prefix: &str,
    middle: &str,
    suffix: &str,
) -> Option<(usize, usize)> {
    let body = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    let (left, right) = body.split_once(middle)?;
    Some((left.parse().ok()?, right.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trade_layout::{TradeScrollbarGeometry, TradeScrollbarPartGeometry};

    fn part(rect: Rect) -> TradeScrollbarPartGeometry {
        TradeScrollbarPartGeometry {
            rect,
            sprite_name: "test".into(),
            border_size: None,
        }
    }

    fn scrollbar() -> PeaceConferenceScrollbarInput {
        PeaceConferenceScrollbarInput {
            geometry: TradeScrollbarGeometry {
                background: part(Rect::new(100, 0, 18, 100)),
                track: part(Rect::new(103, 18, 8, 64)),
                decrease: part(Rect::new(102, 3, 16, 16)),
                increase: part(Rect::new(101, 81, 16, 16)),
                slider: part(Rect::new(101, 16, 14, 18)),
                thumb_base_y: 19,
                thumb_travel_pixels: 46,
                thumb_authored_offset_y: -2,
                smooth_factor_fixed: 25_000,
            },
            step_pixels: 40,
        }
    }

    fn input() -> PeaceConferenceInputRegion {
        PeaceConferenceInputRegion {
            panel: [0, 0, 200, 200],
            available_actions: [0, 0, 100, 100],
            current_and_history: [0, 100, 100, 100],
            contested_popup: None,
            available_max_scroll_px: 100,
            history_max_scroll_px: 100,
            contested_max_scroll_px: 0,
            available_wheel_pixels: None,
            history_wheel_pixels: None,
            contested_wheel_pixels: None,
            available_smooth: true,
            history_smooth: false,
            contested_smooth: false,
            available_smooth_factor_fixed: 25_000,
            history_smooth_factor_fixed: 25_000,
            contested_smooth_factor_fixed: 25_000,
            available_scrollbar: scrollbar(),
            history_scrollbar: scrollbar(),
            contested_scrollbar: None,
        }
    }

    #[test]
    fn popup_occlusion_removes_only_the_exact_overlap() {
        assert_eq!(
            subtract_rect([0, 0, 100, 100], [25, 25, 50, 50]),
            vec![
                [0, 0, 100, 25],
                [0, 75, 100, 25],
                [0, 25, 25, 50],
                [75, 25, 25, 50]
            ]
        );
        assert!(subtract_rect([0, 0, 10, 10], [0, 0, 10, 10]).is_empty());
        assert_eq!(
            subtract_rect([0, 0, 10, 10], [20, 20, 2, 2]),
            vec![[0, 0, 10, 10]]
        );
    }

    #[test]
    fn authored_scrollbar_step_and_smoothing_mutate_real_viewer_state() {
        let input = input();
        let mut viewer = PeaceConferenceViewerState::default();
        viewer
            .apply_scrollbar_action(
                &input,
                PeaceConferenceScrollbarAction::Step(
                    PeaceConferenceScrollRegion::AvailableActions,
                    1,
                ),
            )
            .unwrap();
        assert_eq!(viewer.available_actions_scroll_px, 0);
        viewer.advance_smooth_scroll(&input).unwrap();
        assert_eq!(viewer.available_actions_scroll_px, 10);
        assert_eq!(
            input.scrollbar_action_at(&viewer, (103, 84)).unwrap(),
            Some(PeaceConferenceScrollbarAction::Step(
                PeaceConferenceScrollRegion::AvailableActions,
                1
            ))
        );
    }

    #[test]
    fn contested_popup_blocks_underlying_scrollbar_input() {
        let mut input = input();
        input.contested_popup = Some([90, 70, 50, 30]);
        input.contested_scrollbar = None;
        let viewer = PeaceConferenceViewerState::default();

        assert_eq!(
            input.scrollbar_action_at(&viewer, (103, 84)).unwrap(),
            None,
            "popup interior must occlude the underlying main scrollbar"
        );
        assert_eq!(
            input.scrollbar_action_at(&viewer, (103, 8)).unwrap(),
            Some(PeaceConferenceScrollbarAction::Step(
                PeaceConferenceScrollRegion::AvailableActions,
                -1
            )),
            "the same authored scrollbar remains active outside the popup"
        );
    }

    #[test]
    fn active_compositor_hit_surface_routes_materialized_allocation_and_clips_popup() {
        let text = |id: &str| replay_core::PeaceFinalTextExact {
            authored_identity: id.into(),
            raw_value: id.into(),
            final_text: id.into(),
            final_text_sha256: replay_core::xui08_text_sha256(id),
        };
        let tooltip = replay_core::PeaceTooltipExact {
            primary: text("tooltip"),
            delayed: None,
        };
        let allocation = replay_core::PeaceAllocationPresentationExact {
            source_ordinal: 0,
            stable_allocation_id: "allocation:77:1:4:0".into(),
            turn: 1,
            action_kind: replay_core::PeaceActionKindExact::TakeState,
            actor_participant_id: "participant:1:1".into(),
            beneficiary_country_tag: None,
            beneficiary_country_tag_db_id: None,
            target_country_tag: Some("POL".into()),
            target_country_tag_db_id: Some(2),
            expected_state_count: 1,
            expected_ship_count: 0,
            expected_contender_count: 0,
            target_state_ids: vec![1],
            target_ship_ids: vec![],
            contested_by_participant_ids: vec![],
            action_display: text("action"),
            target_display: text("target"),
            cost: text("cost"),
            bid: text("bid"),
            status: text("status"),
            popup_bid_count_display: text("count"),
            tooltip,
        };
        let group = replay_core::PeaceAllocationGroupPresentationExact {
            source_ordinal: 0,
            stable_group_id: "group:0".into(),
            expected_allocation_count: 1,
            allocation_ids: vec![allocation.stable_allocation_id.clone()],
            flag_texture_path: None,
            target_display: text("group-target"),
            available_state_count_display: text("available"),
            wargoal_count_display: text("wargoals"),
            contested_state_count_display: text("contested"),
        };
        let output = ComposeOutput {
            image: crate::dds::RgbaImage {
                width: 200,
                height: 200,
                pixels: vec![0; 200 * 200 * 4],
            },
            report: Default::default(),
            debug_rects: vec![crate::compose::DebugRect {
                name: "xui08_target_0_allocation_0_Background".into(),
                element_type: "buttonType".into(),
                child_path: vec![0, 0],
                x: 10,
                y: 10,
                w: 100,
                h: 40,
                depth: 2,
            }],
        };
        let groups = vec![(&group, vec![&allocation])];
        let no_contested_allocations = BTreeSet::new();
        assert!(
            hit_targets_from_debug(
                &output,
                &[],
                std::slice::from_ref(&allocation),
                &groups,
                &no_contested_allocations,
                Rect::new(0, 0, 150, 100),
                Rect::new(0, 100, 150, 100),
                Some(Rect::new(60, 0, 50, 100)),
            )
            .is_empty(),
            "ordinary allocations must not create invisible popup selection"
        );

        let contested_allocations =
            BTreeSet::from([allocation.stable_allocation_id.as_str()]);
        let targets = hit_targets_from_debug(
            &output,
            &[],
            std::slice::from_ref(&allocation),
            &groups,
            &contested_allocations,
            Rect::new(0, 0, 150, 100),
            Rect::new(0, 100, 150, 100),
            Some(Rect::new(60, 0, 50, 100)),
        );
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].action,
            PeaceConferenceHitAction::SelectAllocation("allocation:77:1:4:0".into())
        );
        assert!(targets[0].contains((20, 20)));
        assert!(!targets[0].contains((70, 20)));
    }

    #[test]
    fn terminal_tooltip_result_lookup_uses_global_source_ordinal_and_exact_membership() {
        let text = |id: &str| replay_core::PeaceFinalTextExact {
            authored_identity: id.into(),
            raw_value: id.into(),
            final_text: id.into(),
            final_text_sha256: replay_core::xui08_text_sha256(id),
        };
        let tooltip = |id: &str| replay_core::PeaceTooltipExact {
            primary: text(id),
            delayed: None,
        };
        let result = |source_ordinal: u32, stable_result_id: &str| {
            replay_core::PeaceResultPresentationExact {
                source_ordinal,
                stable_result_id: stable_result_id.into(),
                allocation_id: None,
                action_kind: replay_core::PeaceActionKindExact::TakeState,
                actor_country_tag: "GER".into(),
                actor_country_tag_db_id: 1,
                beneficiary_country_tag: None,
                beneficiary_country_tag_db_id: None,
                target_country_tag: Some("POL".into()),
                target_country_tag_db_id: Some(2),
                expected_state_count: 1,
                expected_ship_count: 0,
                target_state_ids: vec![7],
                target_ship_ids: Vec::new(),
                action_display: text("action"),
                target_display: text("target"),
                outcome: text(stable_result_id),
                tooltip: tooltip(stable_result_id),
            }
        };
        let results = vec![result(2, "result:other"), result(7, "result:member")];
        let participant = replay_core::PeaceParticipantPresentationExact {
            source_ordinal: 0,
            stable_participant_id: "participant:GER".into(),
            country_reference: replay_core::PeaceObjectReferenceExact {
                object_type: 1,
                object_id: 1,
            },
            country_tag: "GER".into(),
            country_tag_db_id: 1,
            flag_texture_path: "gfx/flags/GER.tga".into(),
            role: replay_core::PeaceParticipantRoleExact::Victor,
            country_display: text("Germany"),
            score: text("10"),
            score_ratio: text("100%"),
            expected_allocation_count: 0,
            expected_result_count: 1,
            allocation_ids: Vec::new(),
            result_ids: vec!["result:member".into()],
            bid_count_display: text("0"),
            contested_bid_count_display: text("0"),
            tooltip: tooltip("participant"),
        };

        assert_eq!(
            participant_result_by_source_ordinal(&participant, &results, 7)
                .map(|row| row.stable_result_id.as_str()),
            Some("result:member")
        );
        assert!(
            participant_result_by_source_ordinal(&participant, &results, 2).is_none(),
            "a global result outside this participant's exact membership must not bind"
        );
        assert!(
            participant_result_by_source_ordinal(&participant, &results, 0).is_none(),
            "participant-local ordinal zero must not alias global source ordinal seven"
        );
    }

    #[test]
    fn measured_empty_peace_tooltip_is_preserved_and_authenticated() {
        let empty = replay_core::PeaceFinalTextExact {
            authored_identity: "producer-empty".into(),
            raw_value: "".into(),
            final_text: "".into(),
            final_text_sha256: replay_core::xui08_text_sha256(""),
        };
        let exact = replay_core::PeaceConferencePresentationExact {
            schema_version: replay_core::XUI08_SCHEMA_VERSION,
            authority: replay_core::XUI08_AUTHORITY.into(),
            executable_sha256: replay_core::XUI08_EXECUTABLE_SHA256.into(),
            content_manifest_sha256: "1".repeat(64),
            producer_manifest_sha256: replay_core::XUI08_PRODUCER_CONTRACT_MANIFEST_SHA256.into(),
            evidence_manifest_sha256: replay_core::XUI08_EVIDENCE_MANIFEST_SHA256.into(),
            frame_hour: 1,
            capture_sequence: 1,
            passive_global: true,
            ui_open_independent: true,
            recorder_pov_independent: true,
            producer_complete: true,
            previous_record_sha256: None,
            record_sha256: "2".repeat(64),
            observation: replay_core::PeaceConferencePresentationObservationExact::Inactive {
                refresh_sequence: 1,
            },
        };
        let payloads = peace_tooltip_payloads(
            &replay_core::PeaceTooltipExact {
                primary: empty.clone(),
                delayed: Some(empty),
            },
            &exact,
            "row",
            "conference",
            "interface/peaceconferencewindow.gui",
            vec![1],
            "tooltip",
        )
        .unwrap();
        assert!(matches!(
            payloads.primary.rows()[0].availability,
            RowAvailability::MeasuredEmpty
        ));
        assert!(matches!(
            payloads.delayed.unwrap().rows()[1].availability,
            RowAvailability::MeasuredEmpty
        ));
    }
}
