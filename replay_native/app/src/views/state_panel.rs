//! CORE SURFACE 3 (migrated) — the STATE PANEL (`province_state.json` →
//! `countrystateview` ROOT), a Panel-band window opened on a province pick.
//!
//! It owns the selected `state_id` + resolved name (its persistent state). `bind`
//! rebuilds the scene-graph fresh, binds via `bind::bind_state`, lays it out at the
//! fixed panel placement, and returns a `Drawable` that draws the nodes + the owner/
//! controller flags + the buildings/resources gridboxes + scrollbars + the occupation bars. ALL the
//! old `compose.rs` special draws for this surface move HERE (Codex (c)) — no compose
//! branch remains. The Codex §3 block-rect guard is declared so a click on the panel's
//! empty interior is consumed, not a map re-pick.

use bind::{bind_state, BuildingRow, FlagResolver, StateBindReport};
use gui_runtime::draw::{DrawStats, TextureCache};
use gui_runtime::fonts::FontCache;
use gui_runtime::gridbox::{self, GridSpec};
use gui_runtime::scene::{ElementKind, Node};
use gui_runtime::{build_scene, layout, GuiFile, Loc, ManifestContent, Rect};

use crate::host::context::BindContext;
use crate::host::draw::FrameDraw;
use crate::host::input::{PointerButton, ViewInput};
use crate::host::registry::{ViewDescriptor, ViewId};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InputOutcome, InstanceId, RebindContext,
    ViewCommand, ViewInstance, ZBand,
};
use crate::host::FidelityReport;
use crate::register_view;
use widgets::{
    plan_tooltip, Atom as WAtom, Fidelity as WFidelity, Reason as WReason, Tooltip, TooltipPlan,
};

pub const STATE_PANEL_VIEW_ID: ViewId = ViewId("core.state_panel");
const PANEL_FILE: &str = "province_state.json";
const PANEL_X: f32 = 40.0;
const PANEL_Y: f32 = 110.0;
const STATE_BUILDING_ROW_H: f32 = 70.0;
const STATE_BUILDING_CELL_W: f32 = 140.0;
const STATE_BUILDING_SCROLLBAR_W: f32 = 14.0;
const STATE_BUILDING_COLUMNS: usize = 3;
const STATE_RESOURCE_CELL_W: f32 = 23.0;
const STATE_RESOURCE_CELL_H: f32 = 27.0;
const STATE_RESOURCE_COLUMNS: usize = 5;
const STATE_OCCUPATION_BUTTON_SIZE: f32 = 34.0;
const STATE_SCORCHED_EARTH_BUTTON_SIZE: f32 = 34.0;

/// The state panel instance: the selected state + its display name (persistent state).
pub struct StatePanelView {
    pub state_id: u32,
    pub state_name: String,
    building_scroll_y: f32,
    building_scroll_drag: Option<StatePanelScrollDrag>,
    building_row_count: usize,
    resource_row_count: usize,
    hover: Option<StatePanelHover>,
}

impl StatePanelView {
    pub fn new(state_id: u32, state_name: impl Into<String>) -> StatePanelView {
        StatePanelView {
            state_id,
            state_name: state_name.into(),
            building_scroll_y: 0.0,
            building_scroll_drag: None,
            building_row_count: 0,
            resource_row_count: 0,
            hover: None,
        }
    }
}

#[derive(Clone, Copy)]
struct StatePanelScrollDrag {
    pointer_offset_y: f32,
}

#[derive(Clone, Copy)]
struct StatePanelScrollbarGeometry {
    track: Rect,
    thumb: Rect,
    max_scroll: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct StatePanelHover {
    target: StatePanelHoverTarget,
    cursor: (f32, f32),
}

#[derive(Debug, Clone, PartialEq)]
enum StatePanelHoverTarget {
    OwnerFlag,
    ControllerFlag,
    BuildingIndex(usize),
    ResourceIndex(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatePanelBuildingScope {
    State,
    Province,
}

/// The state panel's `Drawable`: nodes + flags + buildings/resources grids + occupation bars.
pub struct StatePanelDrawable {
    roots: Vec<Node>,
    report: StateBindReport,
    panel_file: GuiFile,
    building_scroll_y: f32,
    tooltip: Option<TooltipPlan>,
}

impl Drawable for StatePanelDrawable {
    /// The state panel's extra (non-node) pixel inputs: the owner/controller flag tags,
    /// the building/resource rows (key+level+frame), and the occupation bar fractions.
    /// These are custom draws OUTSIDE the node digest, so digesting them here makes the
    /// panel's flags/gridboxes/bars byte-comparable across a scrub (Codex (c)).
    fn draw_digest(&self) -> String {
        let mut s = format!(
            "STATE_PANEL owner={:?} ctrl={:?} buildings=[",
            self.report.owner_tag, self.report.controller_tag
        );
        for b in self
            .report
            .state_buildings
            .iter()
            .chain(self.report.province_buildings.iter())
        {
            s.push_str(&format!("{}={}@{};", b.key, b.level, b.frame));
        }
        s.push(']');
        let visible = visible_state_building_rows(
            &self.report,
            self.building_scroll_y,
            state_building_viewport_from_roots(&self.roots),
        );
        let max_scroll = max_state_panel_building_scroll(
            state_building_viewport_from_roots(&self.roots),
            state_panel_building_row_count(&self.report),
        );
        s.push_str(&format!(
            " building_scroll={:.0}/{:.0} visible_buildings=[",
            self.building_scroll_y.clamp(0.0, max_scroll),
            max_scroll
        ));
        for b in visible {
            s.push_str(&format!("{};", b.key));
        }
        s.push(']');
        s.push_str(" resources=[");
        for r in &self.report.resource_rows {
            s.push_str(&format!("{}={}@{};", r.key, r.level, r.frame));
        }
        s.push(']');
        match &self.report.occupation {
            Some(o) => s.push_str(&format!(" occ=r{:.2}/c{:.2}", o.resistance, o.compliance)),
            None => s.push_str(" occ=none"),
        }
        s
    }

    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        paint_state_flags(fd, &self.roots, &self.report);
        render_buildings_grid(
            fd,
            &self.roots,
            &self.panel_file,
            &self.report,
            self.building_scroll_y,
        );
        render_resources_grid(fd, &self.roots, &self.panel_file, &self.report);
        render_occupation_bars(fd, &self.roots, &self.report);
    }

    fn tooltip_plan(&self) -> Option<&TooltipPlan> {
        self.tooltip.as_ref()
    }
}

impl ViewInstance for StatePanelView {
    fn view_id(&self) -> ViewId {
        STATE_PANEL_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let stage = ctx.stage;
        let report = bind_state(ctx.snapshot(), &self.state_id.to_string(), &self.state_name);
        let panel_file = stage
            .gui_file(PANEL_FILE)
            .cloned()
            .unwrap_or_else(empty_gui);
        let roots = build_panel_roots(stage, &panel_file, &report, ctx.screen);
        let viewport = state_building_viewport_from_roots(&roots);
        let building_scroll_y = self.building_scroll_y.clamp(
            0.0,
            max_state_panel_building_scroll(viewport, state_panel_building_row_count(&report)),
        );
        let tooltip = state_panel_tooltip_for_hover(
            &report,
            self.hover.as_ref(),
            building_scroll_y,
            ctx.screen,
        );

        let fidelity = panel_fidelity(&report);

        BoundView {
            instance: InstanceId(0),
            view_id: STATE_PANEL_VIEW_ID,
            policy: self.policy(),
            // the Codex §3 guard: the panel root is a pure container (no capturing bg),
            // so without an explicit block rect a click on its empty interior would
            // re-pick a province BEHIND it. Declare the panel's bounds as the block rect.
            hit: HitModel::from_roots(roots.clone()).with_block_rect(panel_block_rect()),
            drawable: Box::new(StatePanelDrawable {
                roots,
                report,
                panel_file,
                building_scroll_y,
                tooltip,
            }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => {
                if state_panel_mutation_control_hit(x, y) {
                    return state_panel_input_consumed(false);
                }
                let viewport = state_building_viewport_fallback();
                let scroll_y = self.building_scroll_y.min(max_state_panel_building_scroll(
                    viewport,
                    self.building_row_count,
                ));
                if let Some(geom) = state_panel_building_scrollbar_geometry(
                    viewport,
                    self.building_row_count,
                    scroll_y,
                ) {
                    if geom.thumb.contains(x, y) {
                        self.building_scroll_drag = Some(StatePanelScrollDrag {
                            pointer_offset_y: y - geom.thumb.y,
                        });
                        let hover_changed = self.clear_hover();
                        return state_panel_input_consumed(hover_changed);
                    }
                    if geom.track.contains(x, y) {
                        let hover_changed = self.clear_hover();
                        return state_panel_input_consumed(hover_changed);
                    }
                }
                InputOutcome::ignored()
            }
            ViewInput::Wheel { x, y, dy } => {
                let viewport = state_building_viewport_fallback();
                if !viewport.contains(x, y) {
                    return InputOutcome::ignored();
                }
                let current = self.building_scroll_y;
                let next = scroll_state_panel_buildings_offset(
                    current,
                    viewport,
                    self.building_row_count,
                    dy,
                );
                let before_hover = self.hover.clone();
                self.building_scroll_y = next;
                self.hover = state_panel_hover_at(
                    x,
                    y,
                    self.building_scroll_y,
                    self.building_row_count,
                    self.resource_row_count,
                );
                state_panel_input_consumed(next != current || self.hover != before_hover)
            }
            ViewInput::PointerMove { x: _, y, drag } if drag => {
                let drag = match self.building_scroll_drag {
                    Some(drag) => drag,
                    None => return InputOutcome::ignored(),
                };
                let viewport = state_building_viewport_fallback();
                let geom = match state_panel_building_scrollbar_geometry(
                    viewport,
                    self.building_row_count,
                    self.building_scroll_y,
                ) {
                    Some(geom) => geom,
                    None => return InputOutcome::ignored(),
                };
                let current = self.building_scroll_y;
                let next =
                    state_panel_building_scroll_from_thumb_y(geom, y - drag.pointer_offset_y);
                self.building_scroll_y = next;
                let hover_changed = self.clear_hover();
                state_panel_input_consumed(next != current || hover_changed)
            }
            ViewInput::PointerMove { x, y, drag: false } => {
                let before = self.hover.clone();
                self.hover = state_panel_hover_at(
                    x,
                    y,
                    self.building_scroll_y,
                    self.building_row_count,
                    self.resource_row_count,
                );
                state_panel_hover_outcome(self.hover != before)
            }
            ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            } => {
                if self.building_scroll_drag.take().is_some() {
                    state_panel_input_consumed(false)
                } else if state_panel_mutation_control_hit(x, y) {
                    state_panel_input_consumed(false)
                } else {
                    InputOutcome::ignored()
                }
            }
            _ => InputOutcome::ignored(),
        }
    }

    fn rebind(&mut self, ctx: &RebindContext<'_>) {
        let report = bind_state(ctx.snapshot, &self.state_id.to_string(), &self.state_name);
        self.building_row_count = state_panel_building_row_count(&report);
        self.resource_row_count = report.resource_rows.len();
        let viewport = state_building_viewport_fallback();
        self.building_scroll_y = self.building_scroll_y.min(max_state_panel_building_scroll(
            viewport,
            self.building_row_count,
        ));
        if state_panel_building_scrollbar_geometry(
            viewport,
            self.building_row_count,
            self.building_scroll_y,
        )
        .is_none()
        {
            self.building_scroll_drag = None;
        }
        if !state_panel_hover_target_valid(
            self.hover.as_ref().map(|h| &h.target),
            self.building_row_count,
            self.resource_row_count,
        ) {
            self.hover = None;
        }
    }

    crate::impl_view_any!();
}

impl StatePanelView {
    fn clear_hover(&mut self) -> bool {
        self.hover.take().is_some()
    }
}

fn state_panel_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
}

fn state_panel_mutation_control_hit(x: f32, y: f32) -> bool {
    state_panel_mutation_control_rects()
        .iter()
        .any(|(_, rect)| rect.contains(x, y))
}

fn state_panel_mutation_control_rects() -> [(&'static str, Rect); 3] {
    [
        (
            "select_law_button",
            Rect {
                x: PANEL_X + 20.0,
                y: PANEL_Y + 185.0 - 8.0 + 68.0,
                w: STATE_OCCUPATION_BUTTON_SIZE,
                h: STATE_OCCUPATION_BUTTON_SIZE,
            },
        ),
        (
            "template_garrison_button",
            Rect {
                x: PANEL_X + 20.0 + 34.0,
                y: PANEL_Y + 185.0 - 8.0 + 69.0,
                w: STATE_OCCUPATION_BUTTON_SIZE,
                h: STATE_OCCUPATION_BUTTON_SIZE,
            },
        ),
        (
            "scorched_earth_state_button",
            Rect {
                x: PANEL_X + 436.0 - 17.0,
                y: PANEL_Y + 413.0,
                w: STATE_SCORCHED_EARTH_BUTTON_SIZE,
                h: STATE_SCORCHED_EARTH_BUTTON_SIZE,
            },
        ),
    ]
}

fn state_panel_hover_outcome(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: false,
        commands: Vec::new(),
        needs_redraw,
    }
}

fn state_panel_building_row_count(report: &StateBindReport) -> usize {
    report.state_buildings.len() + report.province_buildings.len()
}

fn state_panel_building_rows(report: &StateBindReport) -> Vec<&BuildingRow> {
    report
        .state_buildings
        .iter()
        .chain(report.province_buildings.iter())
        .collect()
}

fn state_building_viewport_fallback() -> Rect {
    Rect {
        x: PANEL_X + 153.0,
        y: PANEL_Y + 45.0,
        w: 480.0,
        h: 105.0,
    }
}

fn state_building_viewport_from_roots(roots: &[Node]) -> Rect {
    find_rect(roots, "state_building_entries").unwrap_or_else(state_building_viewport_fallback)
}

fn state_resources_viewport_fallback() -> Rect {
    Rect {
        x: PANEL_X + 26.0 - 44.0,
        y: PANEL_Y + 270.0 + 3.0,
        w: 132.0,
        h: 70.0,
    }
}

fn state_owner_flag_rect_fallback() -> Rect {
    Rect {
        x: PANEL_X + 35.0,
        y: PANEL_Y + 63.0,
        w: 60.0,
        h: 40.0,
    }
}

fn state_controller_flag_rect_fallback() -> Rect {
    Rect {
        x: PANEL_X + 105.0,
        y: PANEL_Y + 63.0,
        w: 41.0,
        h: 26.0,
    }
}

fn state_panel_building_content_height(row_count: usize) -> f32 {
    let grid_rows = row_count.div_ceil(STATE_BUILDING_COLUMNS);
    grid_rows as f32 * STATE_BUILDING_ROW_H
}

fn max_state_panel_building_scroll(viewport: Rect, row_count: usize) -> f32 {
    gridbox::ScrollBar::max_scroll(viewport.h, state_panel_building_content_height(row_count))
}

fn state_panel_building_scrollbar_geometry(
    viewport: Rect,
    row_count: usize,
    scroll_y: f32,
) -> Option<StatePanelScrollbarGeometry> {
    let content_h = state_panel_building_content_height(row_count);
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport.h, content_h);
    if max_scroll <= 0.0 {
        return None;
    }
    let track = Rect {
        x: viewport.x + viewport.w - STATE_BUILDING_SCROLLBAR_W,
        y: viewport.y,
        w: STATE_BUILDING_SCROLLBAR_W,
        h: viewport.h,
    };
    let visible_frac = (viewport.h / content_h).clamp(0.05, 1.0);
    let thumb_h = (track.h * visible_frac).max(16.0).min(track.h);
    let scroll_frac = (scroll_y.clamp(0.0, max_scroll) / max_scroll).clamp(0.0, 1.0);
    let thumb_y = track.y + (track.h - thumb_h).max(0.0) * scroll_frac;
    Some(StatePanelScrollbarGeometry {
        track,
        thumb: Rect {
            x: track.x,
            y: thumb_y,
            w: STATE_BUILDING_SCROLLBAR_W,
            h: thumb_h,
        },
        max_scroll,
    })
}

fn state_panel_building_scroll_from_thumb_y(
    geom: StatePanelScrollbarGeometry,
    thumb_y: f32,
) -> f32 {
    let travel = (geom.track.h - geom.thumb.h).max(0.0);
    if travel <= 0.0 || geom.max_scroll <= 0.0 {
        return 0.0;
    }
    let y = thumb_y.clamp(geom.track.y, geom.track.y + travel);
    ((y - geom.track.y) / travel) * geom.max_scroll
}

fn scroll_state_panel_buildings_offset(
    current: f32,
    viewport: Rect,
    row_count: usize,
    dy: f32,
) -> f32 {
    if dy == 0.0 || !dy.is_finite() {
        return current;
    }
    let rows = dy.abs().ceil().max(1.0);
    let delta = rows * STATE_BUILDING_ROW_H;
    let next = if dy < 0.0 {
        current + delta
    } else {
        (current - delta).max(0.0)
    };
    next.min(max_state_panel_building_scroll(viewport, row_count))
}

fn visible_state_building_rows(
    report: &StateBindReport,
    scroll_y: f32,
    viewport: Rect,
) -> Vec<&BuildingRow> {
    state_panel_building_rows(report)
        .into_iter()
        .enumerate()
        .filter_map(|(idx, row)| {
            let grid_row = idx / STATE_BUILDING_COLUMNS;
            let row_top = grid_row as f32 * STATE_BUILDING_ROW_H;
            let row_bottom = row_top + STATE_BUILDING_ROW_H;
            if row_bottom > scroll_y && row_top < scroll_y + viewport.h {
                Some(row)
            } else {
                None
            }
        })
        .collect()
}

fn state_panel_hover_at(
    x: f32,
    y: f32,
    building_scroll_y: f32,
    building_row_count: usize,
    resource_row_count: usize,
) -> Option<StatePanelHover> {
    let target = if state_owner_flag_rect_fallback().contains(x, y) {
        Some(StatePanelHoverTarget::OwnerFlag)
    } else if state_controller_flag_rect_fallback().contains(x, y) {
        Some(StatePanelHoverTarget::ControllerFlag)
    } else if let Some(idx) =
        state_panel_building_hit_index(x, y, building_scroll_y, building_row_count)
    {
        Some(StatePanelHoverTarget::BuildingIndex(idx))
    } else {
        state_panel_resource_hit_index(x, y, resource_row_count)
            .map(StatePanelHoverTarget::ResourceIndex)
    }?;
    Some(StatePanelHover {
        target,
        cursor: (x, y),
    })
}

fn state_panel_building_hit_index(
    x: f32,
    y: f32,
    scroll_y: f32,
    row_count: usize,
) -> Option<usize> {
    let viewport = state_building_viewport_fallback();
    if row_count == 0 || !viewport.contains(x, y) {
        return None;
    }
    let local_x = x - viewport.x;
    let content_w = STATE_BUILDING_CELL_W * STATE_BUILDING_COLUMNS as f32;
    if local_x < 0.0 || local_x >= content_w {
        return None;
    }
    let col = (local_x / STATE_BUILDING_CELL_W).floor() as usize;
    if col >= STATE_BUILDING_COLUMNS {
        return None;
    }
    let local_y = y - viewport.y + scroll_y;
    if local_y < 0.0 {
        return None;
    }
    let row = (local_y / STATE_BUILDING_ROW_H).floor() as usize;
    let idx = row * STATE_BUILDING_COLUMNS + col;
    (idx < row_count).then_some(idx)
}

fn state_panel_resource_hit_index(x: f32, y: f32, row_count: usize) -> Option<usize> {
    let viewport = state_resources_viewport_fallback();
    if row_count == 0 || !viewport.contains(x, y) {
        return None;
    }
    let col = ((x - viewport.x) / STATE_RESOURCE_CELL_W).floor() as usize;
    if col >= STATE_RESOURCE_COLUMNS {
        return None;
    }
    let row = ((y - viewport.y) / STATE_RESOURCE_CELL_H).floor() as usize;
    let idx = row * STATE_RESOURCE_COLUMNS + col;
    (idx < row_count).then_some(idx)
}

fn state_panel_hover_target_valid(
    target: Option<&StatePanelHoverTarget>,
    building_row_count: usize,
    resource_row_count: usize,
) -> bool {
    match target {
        None => true,
        Some(StatePanelHoverTarget::OwnerFlag | StatePanelHoverTarget::ControllerFlag) => true,
        Some(StatePanelHoverTarget::BuildingIndex(idx)) => *idx < building_row_count,
        Some(StatePanelHoverTarget::ResourceIndex(idx)) => *idx < resource_row_count,
    }
}

fn state_panel_tooltip_for_hover(
    report: &StateBindReport,
    hover: Option<&StatePanelHover>,
    building_scroll_y: f32,
    screen: Rect,
) -> Option<TooltipPlan> {
    let hover = hover?;
    let model = state_panel_tooltip_model(report, &hover.target, building_scroll_y)?;
    Some(plan_tooltip(&model, hover.cursor, screen))
}

fn state_panel_tooltip_model(
    report: &StateBindReport,
    target: &StatePanelHoverTarget,
    _building_scroll_y: f32,
) -> Option<Tooltip> {
    match target {
        StatePanelHoverTarget::OwnerFlag => {
            report.owner_tag.as_ref()?;
            Some(
                Tooltip::new(WAtom::exact("State owner"))
                    .with_row("State", WAtom::exact(report.state_id.clone()))
                    .with_row("Tag", state_field_atom(report, "state_owners")),
            )
        }
        StatePanelHoverTarget::ControllerFlag => {
            report.controller_tag.as_ref()?;
            Some(
                Tooltip::new(WAtom::exact("State controller"))
                    .with_row("State", WAtom::exact(report.state_id.clone()))
                    .with_row("Tag", state_field_atom(report, "controller_flag")),
            )
        }
        StatePanelHoverTarget::BuildingIndex(idx) => {
            let (row, scope) = state_panel_building_row_by_index(report, *idx)?;
            Some(
                Tooltip::new(WAtom::exact(format!("Building: {}", row.key)))
                    .with_row("Level", building_row_atom(row))
                    .with_row(
                        "Scope",
                        WAtom::exact(match scope {
                            StatePanelBuildingScope::State => "state",
                            StatePanelBuildingScope::Province => "province",
                        }),
                    )
                    .with_row("Sprite frame", WAtom::exact(row.frame.to_string())),
            )
        }
        StatePanelHoverTarget::ResourceIndex(idx) => {
            let row = report.resource_rows.get(*idx)?;
            Some(
                Tooltip::new(WAtom::exact(format!("Resource: {}", row.key)))
                    .with_row("Amount", building_row_atom(row))
                    .with_row("Sprite frame", WAtom::exact(row.frame.to_string())),
            )
        }
    }
}

fn state_panel_building_row_by_index(
    report: &StateBindReport,
    idx: usize,
) -> Option<(&BuildingRow, StatePanelBuildingScope)> {
    if let Some(row) = report.state_buildings.get(idx) {
        return Some((row, StatePanelBuildingScope::State));
    }
    let province_idx = idx.checked_sub(report.state_buildings.len())?;
    report
        .province_buildings
        .get(province_idx)
        .map(|row| (row, StatePanelBuildingScope::Province))
}

fn state_field_atom(report: &StateBindReport, element: &str) -> WAtom {
    match report.get(element) {
        Some(field) if !field.not_captured => atom_from_bind(field.value.clone(), field.fidelity),
        _ => WAtom::unavailable_because(WReason::Absent),
    }
}

fn building_row_atom(row: &BuildingRow) -> WAtom {
    atom_from_bind(row.level.clone(), row.fidelity)
}

fn atom_from_bind(value: String, fidelity: bind::Fidelity) -> WAtom {
    match fidelity {
        bind::Fidelity::Exact => WAtom::exact(value),
        bind::Fidelity::Derived => WAtom::from_parts(value, WFidelity::Derived, WReason::Absent),
        bind::Fidelity::Unavailable => WAtom::unavailable_because(WReason::Absent),
    }
}

/// The panel's map-blocking bounding rect (placement + declared size + chrome margin),
/// identical to the old `panel_block_rect`.
fn panel_block_rect() -> Rect {
    Rect {
        x: PANEL_X,
        y: PANEL_Y,
        w: 436.0 + 8.0,
        h: 496.0 + 8.0,
    }
}

/// Build + lay out the panel scene-graph for the bound report — byte-identical to the
/// old `rebuild_panel_roots` (same filter set + placement + text substitution order).
fn build_panel_roots(
    stage: &crate::host::context::Stage,
    panel_file: &GuiFile,
    report: &StateBindReport,
    screen: Rect,
) -> Vec<Node> {
    let cm = ManifestContent {
        manifest: &stage.manifest,
        measure: |_t, _f| None,
    };
    let mut panel: Vec<Node> = build_scene(panel_file, &stage.manifest, &stage.loc)
        .into_iter()
        .filter(|w| w.name == "countrystateview")
        .collect();
    for root in panel.iter_mut() {
        root.orientation = gui_runtime::Anchor::UpperLeft;
        root.origo = gui_runtime::Anchor::UpperLeft;
        root.pos = (PANEL_X, PANEL_Y);
    }
    for fld in &report.fields {
        if fld.element.contains("status_container")
            || fld.element == "state_owners"
            || fld.element == "controller_flag"
        {
            continue;
        }
        for root in panel.iter_mut() {
            bind::apply::set_element_text(root, &fld.element, &fld.value);
        }
    }
    for root in panel.iter_mut() {
        layout(root, screen, None, &cm);
    }
    panel
}

/// Tally the state-panel bound fields into a fidelity report.
fn panel_fidelity(report: &StateBindReport) -> FidelityReport {
    let mut f = FidelityReport::default();
    for fld in &report.fields {
        if fld.not_captured {
            f.unavailable += 1;
        } else {
            match fld.fidelity {
                bind::Fidelity::Exact => f.exact += 1,
                bind::Fidelity::Derived => f.derived += 1,
                bind::Fidelity::Unavailable => f.unavailable += 1,
            }
        }
    }
    f
}

fn empty_gui() -> GuiFile {
    GuiFile::parse(r#"{"view":"empty","windows":[]}"#).expect("empty gui parses")
}

// --- the special draws, moved out of compose.rs into the owning view ---

fn paint_state_flags(fd: &mut FrameDraw, panel: &[Node], report: &StateBindReport) {
    let flag_res = FlagResolver::new(&fd.stage.resolver);
    if let (Some(tag), Some(rect)) = (&report.owner_tag, find_rect(panel, "state_owners")) {
        if let Some(rf) = flag_res.resolve(tag, None) {
            if let Ok(tex) = fd.renderer.load_image(&rf.path) {
                fd.renderer.draw_sprite(&tex, rect.x, rect.y, 41.0, 26.0);
            }
        }
    }
    if let (Some(tag), Some(rect)) = (&report.controller_tag, find_rect(panel, "controller_flag")) {
        if let Some(rf) = flag_res.resolve(tag, None) {
            if let Ok(tex) = fd.renderer.load_image(&rf.path) {
                fd.renderer.draw_sprite(&tex, rect.x, rect.y, 41.0, 26.0);
            }
        }
    }
}

fn render_buildings_grid(
    fd: &mut FrameDraw,
    panel: &[Node],
    panel_file: &GuiFile,
    report: &StateBindReport,
    scroll_y: f32,
) {
    let viewport = match find_rect(panel, "state_building_entries") {
        Some(r) => r,
        None => return,
    };
    let manifest = &fd.stage.manifest;
    let inst = match instantiate_state_panel_building_grid(
        panel_file, report, viewport, manifest, scroll_y,
    ) {
        Some(inst) => inst,
        None => return,
    };
    let mut stats = DrawStats::default();
    let textures: &mut TextureCache = fd.textures;
    let fonts: &mut FontCache = fd.fonts;
    for row in &inst.rows {
        gui_runtime::draw::draw_window(fd.renderer, row, textures, fonts, &mut stats);
    }
    if let Some(sb) = inst.scrollbar {
        if let Some(t) = textures.get(fd.renderer, "GFX_scrollbar_vertical_bg") {
            let border = manifest
                .get("GFX_scrollbar_vertical_bg")
                .and_then(|e| e.border_size)
                .map(|b| [b[0] as f32, b[1] as f32])
                .unwrap_or([0.0, 52.0]);
            fd.renderer
                .draw_nine_slice(t, sb.track.x, sb.track.y, sb.track.w, sb.track.h, border);
        }
        if let Some(t) = textures.get(fd.renderer, "GFX_generic_box_96") {
            fd.renderer
                .draw_sprite(t, sb.thumb.x, sb.thumb.y, sb.thumb.w, sb.thumb.h);
        }
    }
}

fn instantiate_state_panel_building_grid(
    panel_file: &GuiFile,
    report: &StateBindReport,
    viewport: Rect,
    manifest: &assets::SpriteManifest,
    scroll_y: f32,
) -> Option<gridbox::GridInstances> {
    let template = find_template(panel_file, "state_building_entry")?;
    let rows = state_panel_building_rows(report);
    let n = rows.len();
    if n == 0 {
        return None;
    }
    let spec = GridSpec::from_gridbox(
        &stub_named("state_building_entries", viewport),
        &template,
        14.0,
    );
    let content = ManifestContent {
        manifest,
        measure: |_t, _f| None,
    };
    let mut inst = gridbox::instantiate(&template, n, viewport, &spec, scroll_y, &content);
    for (i, row) in inst.rows.iter_mut().enumerate() {
        let b = rows[i];
        bind::apply::set_element_text(row, "level", &b.level);
        gridbox::set_row_icon_frame(row, "building_picture", b.frame);
    }
    Some(inst)
}

fn render_resources_grid(
    fd: &mut FrameDraw,
    panel: &[Node],
    panel_file: &GuiFile,
    report: &StateBindReport,
) {
    let viewport = match find_rect(panel, "state_resources_entries") {
        Some(r) => r,
        None => return,
    };
    let template = match find_template(panel_file, "state_resource_entry") {
        Some(t) => t,
        None => return,
    };
    let rows = &report.resource_rows;
    if rows.is_empty() {
        return;
    }
    let manifest = &fd.stage.manifest;
    let spec = GridSpec::from_gridbox(
        &stub_named("state_resources_entries", viewport),
        &template,
        14.0,
    );
    let content = ManifestContent {
        manifest,
        measure: |_t, _f| None,
    };
    let mut inst = gridbox::instantiate(&template, rows.len(), viewport, &spec, 0.0, &content);
    for (i, row) in inst.rows.iter_mut().enumerate() {
        let res = &rows[i];
        bind::apply::set_element_text(row, "amount", &res.level);
        gridbox::set_row_icon_frame(row, "resource", res.frame);
    }
    let mut stats = DrawStats::default();
    let textures: &mut TextureCache = fd.textures;
    let fonts: &mut FontCache = fd.fonts;
    for row in &inst.rows {
        gui_runtime::draw::draw_window(fd.renderer, row, textures, fonts, &mut stats);
    }
}

fn render_occupation_bars(fd: &mut FrameDraw, panel: &[Node], report: &StateBindReport) {
    let occ = match &report.occupation {
        Some(o) => *o,
        None => return,
    };
    let base = find_rect(panel, "resistance_container").unwrap_or(Rect {
        x: 60.0,
        y: 760.0,
        w: 519.0,
        h: 100.0,
    });
    let bar_w = 81.0_f32;
    let bar_h = 14.0_f32;
    for (i, (gfx, frac)) in [
        (
            "GFX_occupation_state_resistance_bar",
            (occ.resistance / 100.0) as f32,
        ),
        (
            "GFX_occupation_state_compliance_bar",
            (occ.compliance / 100.0) as f32,
        ),
    ]
    .iter()
    .enumerate()
    {
        let y = base.y + 6.0 + i as f32 * 28.0;
        if let Some(t) = fd.textures.get(fd.renderer, gfx) {
            fd.renderer.draw_progress_bar(
                t,
                base.x + 40.0,
                y,
                bar_w,
                bar_h,
                *frac,
                0,
                1,
                [1.0, 1.0, 1.0, 1.0],
            );
        }
    }
}

fn find_rect(roots: &[Node], name: &str) -> Option<Rect> {
    fn walk(n: &Node, name: &str) -> Option<Rect> {
        if n.name == name && n.rect.w > 0.0 && n.rect.h > 0.0 {
            return Some(n.rect);
        }
        for c in &n.children {
            if let Some(r) = walk(c, name) {
                return Some(r);
            }
        }
        None
    }
    roots.iter().find_map(|r| walk(r, name))
}

fn find_template(file: &GuiFile, name: &str) -> Option<Node> {
    let raw = file
        .windows
        .iter()
        .find(|w| w.name.as_deref() == Some(name))?;
    let loc = Loc::new();
    let manifest = assets::SpriteManifest {
        sprites: std::collections::HashMap::new(),
    };
    // template node uses the file's own kinds; the manifest only promotes 9-slice/
    // progressbar, which the building row doesn't need — an empty manifest matches the
    // old `find_template` (which passed the real manifest but the row has no such kind).
    Some(Node::from_raw(raw, &manifest, &loc))
}

fn stub_named(name: &str, rect: Rect) -> Node {
    Node {
        name: name.to_string(),
        raw_kind: "gridBoxType".into(),
        tooltip: None,
        kind: ElementKind::Container { background: None },
        pos: (0.0, 0.0),
        size_w: None,
        size_h: None,
        orientation: gui_runtime::Anchor::UpperLeft,
        origo: gui_runtime::Anchor::UpperLeft,
        clipping: false,
        alwaystransparent: false,
        rect,
        clip: None,
        children: vec![],
    }
}

/// Opened by a province pick (drill) carrying the state id; the binary resolves the
/// name (via map_dir + loc) before opening — but the registry factory can also build a
/// nameless instance from the arg (the binary prefers the named path).
///
/// NO-FABRICATION (Codex): a missing/invalid arg must NOT fabricate a state. State ids
/// are 1-based in the game, so `arg` absent or `<= 0` means "no real state selected": we
/// build a panel on sentinel id `0` with an EMPTY name, so `bind_state` honestly badges
/// the name (and every snapshot field, since `states."0"` never exists) as not-captured
/// "—" — instead of the old `STATE_0`, a fabricated non-empty name `bind_state` marked
/// EXACT/captured. A real drill always supplies a positive id + resolved name.
fn open(arg: Option<i64>) -> Box<dyn ViewInstance> {
    match arg {
        Some(sid) if sid > 0 => {
            let sid = sid as u32;
            Box::new(StatePanelView::new(sid, format!("STATE_{sid}")))
        }
        // no real selection -> sentinel id 0 + empty (not-captured) name, never a
        // fabricated `STATE_0`.
        _ => Box::new(StatePanelView::new(0, String::new())),
    }
}

register_view!(ViewDescriptor {
    id: STATE_PANEL_VIEW_ID,
    gui_root: Some(crate::host::registry::GuiSymbolRef::root(
        PANEL_FILE,
        "countrystateview"
    )),
    required_domains: &["states"],
    route: None, // opened by province pick, not the menu
    shortcut: None,
    open,
    policy: HostPolicy::window(ZBand::Panel),
});

register_template!(crate::host::registry::TemplateDescriptor {
    id: ViewId("core.state_building_entry"),
    gui_template: crate::host::registry::GuiSymbolRef::template(PANEL_FILE, "state_building_entry"),
    required_domains: &["states"],
});

pub fn link() {}

use crate::register_template;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::{BindContext, Stage};
    use crate::host::view::RebindContext;
    use crate::views::digest::bound_digest;
    use assets::resolver::GameResolver;
    use assets::SpriteManifest;
    use std::collections::HashMap;

    fn panel_gui() -> GuiFile {
        GuiFile::parse(
            r#"{"view":"p","windows":[
              {"type":"containerWindowType","name":"countrystateview","position":{"x":0,"y":0},
               "size":{"width":436,"height":496},
               "children":[
                 {"type":"instantTextBoxType","name":"state_name","position":{"x":10,"y":4},
                  "orientation":"UPPER_LEFT","text":{"text":"NAME","font":"hoi_16mbs"}},
                 {"type":"containerWindowType","name":"state_owners",
                  "position":{"x":35,"y":63},"orientation":"UPPER_LEFT",
                  "size":{"width":60,"height":40}},
                  {"type":"buttonType","name":"controller_flag",
                   "position":{"x":105,"y":63},"orientation":"UPPER_LEFT",
                   "size":{"width":41,"height":26},"sprite":"GFX_flag_small2"},
                  {"type":"containerWindowType","name":"resistance_container",
                   "position":{"x":20,"y":185},"orientation":"UPPER_LEFT",
                   "size":{"width":519,"height":100},
                   "children":[
                     {"type":"containerWindowType","name":"law_and_template_container",
                      "position":{"x":0,"y":-8},"orientation":"UPPER_LEFT",
                      "size":{"width":0,"height":0},
                      "children":[
                        {"type":"buttonType","name":"select_law_button",
                         "position":{"x":0,"y":68},"orientation":"UPPER_LEFT",
                         "size":{"width":34,"height":34},"sprite":"GFX_occupation_state_select_law_button"},
                        {"type":"buttonType","name":"template_garrison_button",
                         "position":{"x":34,"y":69},"orientation":"UPPER_LEFT",
                         "size":{"width":34,"height":34},"sprite":"GFX_unitlist_unitinfo_button_small"}
                      ]}
                   ]},
                  {"type":"buttonType","name":"scorched_earth_state_button",
                   "position":{"x":-17,"y":413},"orientation":"UPPER_RIGHT",
                   "size":{"width":34,"height":34},"sprite":"GFX_scorched_earth_state"},
                  {"type":"containerWindowType","name":"state_building_entries",
                   "position":{"x":153,"y":45},"orientation":"UPPER_LEFT",
                   "size":{"width":480,"height":105}},
                  {"type":"containerWindowType","name":"state_shared_slot_building_entries",
                   "position":{"x":150,"y":170},"orientation":"UPPER_LEFT",
                   "size":{"width":480,"height":90}},
                 {"type":"containerWindowType","name":"state_resources_entries",
                  "position":{"x":-18,"y":273},"orientation":"UPPER_LEFT",
                  "size":{"width":132,"height":70}}
               ]},
              {"type":"containerWindowType","name":"state_building_entry","position":{"x":0,"y":0},
               "size":{"width":140,"height":70},
               "children":[
                 {"type":"instantTextBoxType","name":"level","position":{"x":2,"y":2},
                  "orientation":"UPPER_LEFT","text":{"text":"0","font":"hoi_16mbs"}},
                 {"type":"iconType","name":"building_picture","position":{"x":20,"y":2},
                  "orientation":"UPPER_LEFT","spriteType":"GFX_buildings_strip"}
               ]}
              ,
              {"type":"containerWindowType","name":"state_shared_slot_building_entry","position":{"x":4,"y":-15},
               "size":{"width":70,"height":70},
               "children":[
                 {"type":"iconType","name":"build_slot_bg","position":{"x":0,"y":0},
                  "orientation":"UPPER_LEFT","sprite":"GFX_build_slot_bg"},
                 {"type":"iconType","name":"building_picture","position":{"x":20,"y":2},
                  "orientation":"UPPER_LEFT","spriteType":"GFX_buildings_strip"},
                 {"type":"buttonType","name":"remove","position":{"x":40,"y":29},
                  "orientation":"UPPER_LEFT","size":{"width":24,"height":24},
                  "sprite":"GFX_cancel_conversion_button"}
               ]}
              ,
              {"type":"containerWindowType","name":"state_resource_entry","position":{"x":0,"y":0},
               "size":{"width":23,"height":27},
               "children":[
                 {"type":"iconType","name":"resource","position":{"x":53,"y":11},
                  "orientation":"UPPER_LEFT","spriteType":"GFX_resources_strip"},
                 {"type":"instantTextBoxType","name":"amount","position":{"x":70,"y":11},
                  "orientation":"UPPER_LEFT","text":{"text":"0","font":"hoi_20b"}}
               ]}
            ]}"#,
        )
        .unwrap()
    }

    fn stage() -> Stage {
        let mut gui_files = HashMap::new();
        gui_files.insert(PANEL_FILE.to_string(), panel_gui());
        Stage {
            resolver: GameResolver::new(None, None),
            manifest: SpriteManifest {
                sprites: HashMap::new(),
            },
            loc: Loc::new(),
            gui_files,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
        }
    }

    fn snap_with_buildings(
        state_buildings: serde_json::Value,
        province_buildings: serde_json::Value,
    ) -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12",
            "states": {
                "64": {
                    "owner": "GER",
                    "controller": "GER",
                    "buildings": state_buildings,
                    "province_buildings": province_buildings,
                    "resources": {"steel": 12, "oil": 3}
                }
            }
        }))
        .unwrap()
    }

    fn overflow_snap() -> snapshot::Snapshot {
        snap_with_buildings(
            serde_json::json!({
                "infrastructure": 5,
                "industrial_complex": 3,
                "arms_factory": 7,
                "dockyard": 2,
                "air_base": 4,
                "anti_air_building": 1,
                "radar_station": 1,
                "rocket_site": 1,
                "nuclear_reactor": 1,
                "synthetic_refinery": 2,
                "fuel_silo": 3
            }),
            serde_json::json!({
                "naval_base": 5,
                "rail_way": 4,
                "supply_node": 1,
                "naval_supply_hub": 1,
                "coastal_bunker": 2,
                "bunker": 3,
                "air_facility": 1,
                "land_facility": 1,
                "floating_harbor": 1
            }),
        )
    }

    fn tiny_snap() -> snapshot::Snapshot {
        snap_with_buildings(
            serde_json::json!({
                "infrastructure": 5,
                "industrial_complex": 3,
                "arms_factory": 7
            }),
            serde_json::json!({}),
        )
    }

    fn full_resource_snap() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12",
            "states": {
                "64": {
                    "buildings": {},
                    "province_buildings": {},
                    "resources": {
                        "aluminium": 1,
                        "chromium": 2,
                        "oil": 3,
                        "rubber": 4,
                        "steel": 5,
                        "tungsten": 6
                    }
                }
            }
        }))
        .unwrap()
    }

    fn ctx<'a>(stage: &'a Stage, snap: &'a snapshot::Snapshot) -> BindContext<'a> {
        BindContext {
            stage,
            snapshot: snap,
            country: "GER",
            screen: Rect {
                x: 0.0,
                y: 0.0,
                w: 1600.0,
                h: 900.0,
            },
            date: "1940.1.1.12",
            cursor: (0.0, 0.0),
            map_camera: None,
        }
    }

    fn rebind_ctx<'a>(snap: &'a snapshot::Snapshot) -> RebindContext<'a> {
        RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit: false,
        }
    }

    fn visible_digest(view: &StatePanelView, stage: &Stage, snap: &snapshot::Snapshot) -> String {
        bound_digest(&view.bind(&ctx(stage, snap)))
    }

    fn building_scroll_pair(digest: &str) -> (i32, i32) {
        let tail = digest
            .split(" building_scroll=")
            .nth(1)
            .expect("state panel building scroll digest");
        let pair = tail.split(' ').next().expect("scroll pair");
        let mut parts = pair.split('/');
        let cur = parts.next().unwrap().parse::<i32>().unwrap();
        let max = parts.next().unwrap().parse::<i32>().unwrap();
        (cur, max)
    }

    #[test]
    fn state_panel_owner_flag_hover_emits_factual_tooltip_and_clears() {
        let stage = stage();
        let snap = overflow_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        view.rebind(&rebind_ctx(&snap));

        let rect = state_owner_flag_rect_fallback();
        let out = view.on_input(&ViewInput::PointerMove {
            x: rect.x + 4.0,
            y: rect.y + 4.0,
            drag: false,
        });
        assert!(!out.consumed);
        assert!(out.needs_redraw);
        let bound = view.bind(&ctx(&stage, &snap));
        let tip = bound.tooltip_plan().expect("owner flag tooltip").digest();
        assert!(tip.contains("State owner"), "{tip}");
        assert!(tip.contains("Tag"), "{tip}");
        assert!(tip.contains("GER"), "{tip}");
        assert!(tip.contains("64"), "{tip}");

        let click = view.on_input(&ViewInput::PointerDown {
            x: rect.x + 4.0,
            y: rect.y + 4.0,
            button: PointerButton::Left,
        });
        assert!(!click.consumed);
        assert!(!click.needs_redraw);
        assert!(
            view.bind(&ctx(&stage, &snap)).tooltip_plan().is_some(),
            "ordinary non-scrollbar clicks must not clear hover without a redraw"
        );

        let out = view.on_input(&ViewInput::PointerMove {
            x: 0.0,
            y: 0.0,
            drag: false,
        });
        assert!(!out.consumed);
        assert!(out.needs_redraw);
        assert!(
            view.bind(&ctx(&stage, &snap)).tooltip_plan().is_none(),
            "moving off the state panel clears the passive tooltip"
        );
    }

    #[test]
    fn state_panel_building_hover_is_scroll_aware_and_factual() {
        let stage = stage();
        let snap = overflow_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        view.rebind(&rebind_ctx(&snap));

        let viewport = state_building_viewport_fallback();
        let out = view.on_input(&ViewInput::PointerMove {
            x: viewport.x + 10.0,
            y: viewport.y + 10.0,
            drag: false,
        });
        assert!(!out.consumed);
        assert!(out.needs_redraw);
        let tip = view
            .bind(&ctx(&stage, &snap))
            .tooltip_plan()
            .expect("building tooltip")
            .digest();
        assert!(tip.contains("Building: infrastructure"), "{tip}");
        assert!(tip.contains("Level"), "{tip}");
        assert!(tip.contains("5"), "{tip}");
        assert!(tip.contains("Scope"), "{tip}");
        assert!(tip.contains("state"), "{tip}");

        let out = view.on_input(&ViewInput::Wheel {
            x: viewport.x + 10.0,
            y: viewport.y + 10.0,
            dy: -1.0,
        });
        assert!(out.consumed);
        assert!(out.needs_redraw);
        let tip = view
            .bind(&ctx(&stage, &snap))
            .tooltip_plan()
            .expect("scrolled building tooltip")
            .digest();
        assert!(
            tip.contains("Building: dockyard"),
            "same cursor after one row of scroll should hover the next backing row: {tip}"
        );
        assert!(tip.contains("2"), "{tip}");
    }

    #[test]
    fn state_panel_resource_hover_emits_amount_tooltip() {
        let stage = stage();
        let snap = full_resource_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        view.rebind(&rebind_ctx(&snap));

        let viewport = state_resources_viewport_fallback();
        let out = view.on_input(&ViewInput::PointerMove {
            x: viewport.x + 5.0,
            y: viewport.y + 5.0,
            drag: false,
        });
        assert!(!out.consumed);
        assert!(out.needs_redraw);
        let tip = view
            .bind(&ctx(&stage, &snap))
            .tooltip_plan()
            .expect("resource tooltip")
            .digest();
        assert!(tip.contains("Resource: aluminium"), "{tip}");
        assert!(tip.contains("Amount"), "{tip}");
        assert!(tip.contains("1"), "{tip}");
    }

    #[test]
    fn state_panel_mutation_controls_are_replay_disabled_noops() {
        let stage = stage();
        let snap = overflow_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        view.rebind(&rebind_ctx(&snap));

        for name in [
            "select_law_button",
            "template_garrison_button",
            "scorched_earth_state_button",
        ] {
            let before = visible_digest(&view, &stage, &snap);
            let bound = view.bind(&ctx(&stage, &snap));
            let rect = find_rect(&bound.hit.roots, name).expect("test GUI mutation control");
            let x = rect.x + rect.w * 0.5;
            let y = rect.y + rect.h * 0.5;

            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(down.consumed, "{name} pointer-down must be consumed");
            assert!(
                down.commands.is_empty(),
                "{name} pointer-down must emit no replay command"
            );
            assert!(
                !down.needs_redraw,
                "{name} pointer-down is a disabled no-op"
            );

            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(up.consumed, "{name} pointer-up must be consumed");
            assert!(
                up.commands.is_empty(),
                "{name} pointer-up must emit no replay command"
            );
            assert!(!up.needs_redraw, "{name} pointer-up is a disabled no-op");
            assert_eq!(
                before,
                visible_digest(&view, &stage, &snap),
                "{name} disabled click must leave the bound digest unchanged"
            );
        }
    }

    #[test]
    fn state_panel_shared_slot_remove_buttons_are_not_instantiated_or_claimed_yet() {
        let stage = stage();
        let snap = overflow_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        view.rebind(&rebind_ctx(&snap));

        let panel_file = stage
            .gui_file(PANEL_FILE)
            .expect("test stage has province_state GUI");
        assert!(
            find_template(panel_file, "state_shared_slot_building_entry").is_some(),
            "test GUI must define the shared-slot template before proving it is unrendered"
        );
        let bound = view.bind(&ctx(&stage, &snap));
        let shared_viewport = find_rect(&bound.hit.roots, "state_shared_slot_building_entries")
            .expect("test GUI must expose the shared-slot grid viewport");
        assert!(
            find_rect(&bound.hit.roots, "remove").is_none(),
            "bound state-panel roots must not contain uninstantiated template leaves"
        );

        let report = bind_state(&snap, "64", "STATE_64");
        let state_viewport = state_building_viewport_from_roots(&bound.hit.roots);
        let rendered_state_rows = instantiate_state_panel_building_grid(
            panel_file,
            &report,
            state_viewport,
            &stage.manifest,
            0.0,
        )
        .expect("state-building rows are rendered");
        assert!(
            rendered_state_rows
                .rows
                .iter()
                .all(|row| find_rect(std::slice::from_ref(row), "remove").is_none()),
            "current native building renderer still instantiates state_building_entry rows, not shared-slot rows"
        );

        let shared_template = find_template(panel_file, "state_shared_slot_building_entry")
            .expect("shared-slot template exists in the test GUI");
        let shared_spec = GridSpec::from_gridbox(
            &stub_named("state_shared_slot_building_entries", shared_viewport),
            &shared_template,
            14.0,
        );
        let content = ManifestContent {
            manifest: &stage.manifest,
            measure: |_t, _f| None,
        };
        let would_be_shared_rows = gridbox::instantiate(
            &shared_template,
            1,
            shared_viewport,
            &shared_spec,
            0.0,
            &content,
        );
        let remove_rect = find_rect(&would_be_shared_rows.rows, "remove")
            .expect("the shared-slot template's remove leaf has visible geometry if instantiated");
        let before = visible_digest(&view, &stage, &snap);

        let down = view.on_input(&ViewInput::PointerDown {
            x: remove_rect.x + remove_rect.w * 0.5,
            y: remove_rect.y + remove_rect.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(
            !down.consumed,
            "would-be shared-slot remove geometry must stay unclaimed until the row is rendered"
        );
        assert!(down.commands.is_empty());
        assert!(!down.needs_redraw);

        let up = view.on_input(&ViewInput::PointerUp {
            x: remove_rect.x + remove_rect.w * 0.5,
            y: remove_rect.y + remove_rect.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(
            !up.consumed,
            "pointer-up over would-be remove geometry must also stay unclaimed"
        );
        assert!(up.commands.is_empty());
        assert!(!up.needs_redraw);
        assert_eq!(
            before,
            visible_digest(&view, &stage, &snap),
            "unrendered shared-slot remove target must not affect state-panel digest"
        );
    }

    #[test]
    fn state_panel_disabled_noop_does_not_claim_neighboring_panel_space() {
        let stage = stage();
        let snap = overflow_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        view.rebind(&rebind_ctx(&snap));
        let before = visible_digest(&view, &stage, &snap);

        let out = view.on_input(&ViewInput::PointerDown {
            x: PANEL_X + 150.0,
            y: PANEL_Y + 265.0,
            button: PointerButton::Left,
        });
        assert!(!out.consumed);
        assert!(out.commands.is_empty());
        assert!(!out.needs_redraw);
        assert_eq!(
            before,
            visible_digest(&view, &stage, &snap),
            "neighboring non-control panel space must remain handled only by host block rects"
        );
    }

    #[test]
    fn state_panel_stale_hover_clears_on_rebind_without_fabrication() {
        let stage = stage();
        let snap_a = overflow_snap();
        let snap_b = tiny_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        let viewport = state_building_viewport_fallback();

        view.rebind(&rebind_ctx(&snap_a));
        view.on_input(&ViewInput::Wheel {
            x: viewport.x + 10.0,
            y: viewport.y + 10.0,
            dy: -1.0,
        });
        let tip = view
            .bind(&ctx(&stage, &snap_a))
            .tooltip_plan()
            .expect("scrolled hover exists before shrink")
            .digest();
        assert!(tip.contains("Building: dockyard"), "{tip}");

        view.rebind(&rebind_ctx(&snap_b));
        assert!(
            view.bind(&ctx(&stage, &snap_b)).tooltip_plan().is_none(),
            "hover pointing past the smaller building list must clear, not fall back to another row"
        );
    }

    #[test]
    fn state_panel_building_wheel_scroll_changes_visible_rows_and_clamps() {
        let stage = stage();
        let snap = overflow_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        view.rebind(&rebind_ctx(&snap));

        let initial = visible_digest(&view, &stage, &snap);
        assert!(
            initial.contains("building_scroll=0/385 visible_buildings=[infrastructure;industrial_complex;arms_factory;dockyard;air_base;anti_air_building;]"),
            "{initial}"
        );

        let viewport = state_building_viewport_fallback();
        let out = view.on_input(&ViewInput::Wheel {
            x: viewport.x + 10.0,
            y: viewport.y + 10.0,
            dy: -1.0,
        });
        assert!(out.consumed);
        assert!(out.needs_redraw);
        let scrolled = visible_digest(&view, &stage, &snap);
        assert!(
            scrolled.contains("building_scroll=70/385 visible_buildings=[dockyard;air_base;anti_air_building;radar_station;rocket_site;nuclear_reactor;]"),
            "{scrolled}"
        );

        let out = view.on_input(&ViewInput::Wheel {
            x: viewport.x + 10.0,
            y: viewport.y + 10.0,
            dy: -99.0,
        });
        assert!(out.consumed);
        assert!(out.needs_redraw);
        let clamped = visible_digest(&view, &stage, &snap);
        let (cur, max) = building_scroll_pair(&clamped);
        assert_eq!(cur, max, "huge wheel down must clamp: {clamped}");

        let out = view.on_input(&ViewInput::Wheel {
            x: viewport.x + 10.0,
            y: viewport.y + 10.0,
            dy: -1.0,
        });
        assert!(out.consumed);
        assert!(
            !out.needs_redraw,
            "already-clamped wheel hit is consumed but inert"
        );
    }

    #[test]
    fn state_panel_building_scrollbar_drag_changes_visible_rows_and_clamps() {
        let stage = stage();
        let snap = overflow_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        view.rebind(&rebind_ctx(&snap));

        let viewport = state_building_viewport_fallback();
        let geom = state_panel_building_scrollbar_geometry(viewport, view.building_row_count, 0.0)
            .expect("overflowing building grid has scrollbar");
        let down = view.on_input(&ViewInput::PointerDown {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.thumb.y + geom.thumb.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(down.consumed);
        assert!(!down.needs_redraw);
        assert!(view.building_scroll_drag.is_some());

        let moved = view.on_input(&ViewInput::PointerMove {
            x: geom.track.x + 1.0,
            y: geom.track.y + geom.track.h + 200.0,
            drag: true,
        });
        assert!(moved.consumed);
        assert!(moved.needs_redraw);
        let bottom = visible_digest(&view, &stage, &snap);
        let (cur, max) = building_scroll_pair(&bottom);
        assert_eq!(cur, max, "dragging below track must clamp to max: {bottom}");

        let up = view.on_input(&ViewInput::PointerUp {
            x: geom.track.x + 1.0,
            y: geom.track.y + geom.track.h + 200.0,
            button: PointerButton::Left,
        });
        assert!(up.consumed);
        assert!(!up.needs_redraw);
        assert!(view.building_scroll_drag.is_none());

        let stale = view.on_input(&ViewInput::PointerMove {
            x: geom.track.x + 1.0,
            y: geom.track.y,
            drag: true,
        });
        assert!(!stale.consumed, "drag capture must clear on pointer-up");
    }

    #[test]
    fn state_panel_building_scrollbar_track_click_is_inert_consumed() {
        let stage = stage();
        let snap = overflow_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        view.rebind(&rebind_ctx(&snap));
        let before = visible_digest(&view, &stage, &snap);

        let viewport = state_building_viewport_fallback();
        let geom = state_panel_building_scrollbar_geometry(viewport, view.building_row_count, 0.0)
            .expect("overflowing building grid has scrollbar");
        let out = view.on_input(&ViewInput::PointerDown {
            x: geom.track.x + geom.track.w * 0.5,
            y: geom.thumb.y + geom.thumb.h + 6.0,
            button: PointerButton::Left,
        });
        assert!(out.consumed);
        assert!(!out.needs_redraw);
        assert_eq!(
            before,
            visible_digest(&view, &stage, &snap),
            "track click must not scroll or leak into a row/map action"
        );
    }

    #[test]
    fn state_panel_building_wheel_outside_grid_is_inert() {
        let stage = stage();
        let snap = overflow_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        view.rebind(&rebind_ctx(&snap));
        let before = visible_digest(&view, &stage, &snap);

        let viewport = state_building_viewport_fallback();
        let out = view.on_input(&ViewInput::Wheel {
            x: viewport.x - 1.0,
            y: viewport.y + 10.0,
            dy: -1.0,
        });
        assert!(!out.consumed);
        assert!(!out.needs_redraw);
        assert_eq!(before, visible_digest(&view, &stage, &snap));
    }

    #[test]
    fn state_panel_building_scroll_rebind_clamps_and_rebuilds_visible_rows() {
        let stage = stage();
        let snap_a = overflow_snap();
        let snap_b = tiny_snap();
        let mut view = StatePanelView::new(64, "STATE_64");
        let viewport = state_building_viewport_fallback();

        view.rebind(&rebind_ctx(&snap_a));
        view.on_input(&ViewInput::Wheel {
            x: viewport.x + 10.0,
            y: viewport.y + 10.0,
            dy: -1.0,
        });
        assert_eq!(view.building_scroll_y, 70.0);
        view.rebind(&rebind_ctx(&snap_a));
        assert_eq!(view.building_scroll_y, 70.0, "valid scroll survives rebind");

        let geom = state_panel_building_scrollbar_geometry(viewport, view.building_row_count, 70.0)
            .expect("overflowing building grid has scrollbar");
        view.on_input(&ViewInput::PointerDown {
            x: geom.thumb.x + 1.0,
            y: geom.thumb.y + 1.0,
            button: PointerButton::Left,
        });
        assert!(view.building_scroll_drag.is_some());

        view.rebind(&rebind_ctx(&snap_b));
        assert_eq!(
            view.building_scroll_y, 0.0,
            "smaller snapshot clamps scroll"
        );
        assert!(
            view.building_scroll_drag.is_none(),
            "stale drag clears when overflow disappears"
        );
        let b_digest = visible_digest(&view, &stage, &snap_b);
        assert!(
            b_digest.contains(
                "building_scroll=0/0 visible_buildings=[infrastructure;industrial_complex;arms_factory;]"
            ),
            "{b_digest}"
        );

        view.rebind(&rebind_ctx(&snap_a));
        let a_digest = visible_digest(&view, &stage, &snap_a);
        assert!(
            a_digest.contains("building_scroll=0/385 visible_buildings=[infrastructure;industrial_complex;arms_factory;dockyard;air_base;anti_air_building;]"),
            "{a_digest}"
        );
    }

    #[test]
    fn state_panel_resource_grid_does_not_need_scroll_for_known_resources() {
        let stage = stage();
        let snap = full_resource_snap();
        let report = bind_state(&snap, "64", "STATE_64");
        assert_eq!(
            report.resource_rows.len(),
            6,
            "the game state resources currently expose six ordered resource rows"
        );

        let view = StatePanelView::new(64, "STATE_64");
        let bound = view.bind(&ctx(&stage, &snap));
        let viewport =
            find_rect(&bound.hit.roots, "state_resources_entries").expect("resource grid viewport");
        let template = find_template(
            stage.gui_file(PANEL_FILE).expect("panel gui"),
            "state_resource_entry",
        )
        .expect("resource row template");
        let spec = GridSpec::from_gridbox(
            &stub_named("state_resources_entries", viewport),
            &template,
            14.0,
        );
        let grid_rows = report.resource_rows.len().div_ceil(spec.columns.max(1));
        let content_height = grid_rows as f32 * spec.cell_h;

        assert_eq!(spec.columns, 5);
        assert_eq!(grid_rows, 2);
        assert!(
            gridbox::ScrollBar::max_scroll(viewport.h, content_height) <= 0.0,
            "six resources fit in the measured resource grid without a scrollbar"
        );
    }

    /// NO-FABRICATION (Codex): the registry factory `open(None)` (the generic
    /// command/registry open path with no drill arg) must NOT fabricate a `STATE_0`
    /// non-existent state. It builds a sentinel id `0` with an EMPTY name, so
    /// `bind_state` honestly badges the name as not-captured (state ids are 1-based, so
    /// `states."0"` never exists and every field resolves to "—"), never a fabricated
    /// captured value.
    #[test]
    fn open_with_no_arg_does_not_fabricate_state_zero() {
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12",
            "states": { "64": { "owner": "GER", "victory_points_total": 5 } }
        }))
        .unwrap();

        // the factory must not invent a "STATE_0" name.
        let inst = open(None);
        let v = inst
            .as_any()
            .downcast_ref::<StatePanelView>()
            .expect("state panel view");
        assert_eq!(v.state_id, 0);
        assert_eq!(
            v.state_name, "",
            "open(None) must NOT fabricate a STATE_0 name"
        );

        // and binding the sentinel state reports the name as NOT-CAPTURED (the
        // not-captured glyph), never a fabricated exact value.
        let report = bind_state(&snap, &v.state_id.to_string(), &v.state_name);
        let name_field = report
            .fields
            .iter()
            .find(|f| f.element == "state_name")
            .expect("state_name field");
        assert!(
            name_field.not_captured,
            "the empty sentinel name must badge not-captured, not a fabricated STATE_0"
        );
        assert_ne!(name_field.value, "STATE_0");

        // a NEGATIVE / zero arg is treated the same (no real selection).
        let neg = open(Some(-7));
        let nv = neg.as_any().downcast_ref::<StatePanelView>().unwrap();
        assert_eq!(nv.state_name, "");
        assert_eq!(
            open(Some(0))
                .as_any()
                .downcast_ref::<StatePanelView>()
                .unwrap()
                .state_name,
            ""
        );

        // a real positive arg still builds a named instance (the documented fallback).
        let pos = open(Some(64));
        let pv = pos.as_any().downcast_ref::<StatePanelView>().unwrap();
        assert_eq!(pv.state_id, 64);
        assert_eq!(pv.state_name, "STATE_64");
    }
}
