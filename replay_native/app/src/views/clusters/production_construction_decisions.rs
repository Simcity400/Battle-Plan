//! CLUSTER 2b — production_construction_decisions (3 views) [gridbox].
//!
//! Three array→grid surfaces, bound per `replay_ui/BINDING_MAP_production_construction_decisions.md`:
//!   * **production** — `countryproductionlineview` ROOT: the operational-factory summary
//!     (`economy.factories` split + the resource strip) + the production-line rows
//!     (`economy._production_lines` / the live `production.{military,naval}_lines` channel),
//!     instantiated by the `item_grid` row factory into `production_military_line_entry` /
//!     `production_naval_line_entry`.
//!   * **construction** — `countryconstructionsview` ROOT: the available/local/traded factory
//!     summary + the construction-queue rows (`economy.construction_queue` save channel /
//!     the live `production.construction_queue`), instantiated into `production_building_line_entry`.
//!   * **decisions** — `countrydecisionview` ROOT: the active TIMED-decision rows
//!     (`decisions.timed_live`), instantiated into `timed_decision_item`. The one-shot
//!     availability catalog + the `decision_status` extraction gap badge `unavailable`.
//!
//! THE NO-FABRICATION CONTRACT. This is a CLUSTER file, so the source-scanning guard
//! (`views::digest::no_fabricated_default_in_cluster_views`) forbids `unwrap_or(0)` /
//! `unwrap_or_default()` / the raw `ctx.snapshot()` seam. Every field is read through
//! [`BindContext::country_field`] → [`Captured`] and rendered through [`DisplayAtom`], so a
//! blocked / absent / null field renders the not-captured glyph "—" — NEVER a fabricated 0.
//! Genuinely runtime-computed and live-channel-resisted fields are badged `unavailable`.
//!
//! THE ROW MODEL (Codex Q5). A `gridBoxType` is a row factory: the ROOT scene tree (the
//! static chrome + the summary text boxes) is fixed regardless of how many array elements
//! exist; the per-element rows are instantiated from a TEMPLATE window and CUSTOM-DRAWN by
//! the view's `Drawable` (mirroring `state_panel`'s `render_buildings_grid`). So the
//! element-accounting target is the `.gui`'s static element set (proven == its own
//! `stats.element_type_counts`), and the per-row data lives in the drawable digest +
//! fidelity report — not as extra ROOT nodes.

use gui_runtime::draw::{DrawStats, TextureCache};
use gui_runtime::fonts::FontCache;
use gui_runtime::gridbox::{self, GridSpec};
use gui_runtime::scene::{ElementKind, Node};
use gui_runtime::{build_scene, layout, GuiFile, Loc, ManifestContent, Rect};

use bind::Fidelity;

use crate::host::context::{BindContext, Captured, DisplayAtom, FidelityReport};
use crate::host::draw::FrameDraw;
use crate::host::input::{PointerButton, ViewInput};
use crate::host::registry::{
    GuiSymbolRef, MenuCluster, MenuRoute, TemplateDescriptor, ViewDescriptor, ViewId,
};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InputOutcome, InstanceId, RebindContext,
    ViewCommand, ViewInstance, ZBand,
};
use crate::{register_template, register_view};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

// =====================================================================================
// Shared no-fabrication helpers (read via Captured, render via DisplayAtom — never a 0).
// =====================================================================================

/// The not-captured glyph (the only thing a blocked/absent/null field renders).
const DASH: &str = "—";

/// Read a JSON value out of an array ELEMENT as a [`Captured`] (preserving the
/// Absent/Null/Measured trichotomy at the leaf), so a per-row field is no-fabrication too.
fn elem_field<'a>(elem: &'a serde_json::Value, key: &str) -> Captured<&'a serde_json::Value> {
    crate::host::context::classify_value(elem.get(key))
}

/// Format a measured number as a truncated integer string. Only ever called on a
/// `Measured` value via [`Captured::display_with_fidelity`], so it cannot fabricate.
fn fmt_int(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) => format!("{}", f.trunc() as i64),
        None => v
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| DASH.to_string()),
    }
}

/// Format a measured value as its raw string token (equipment archetype, building
/// template). Returns "—" only if the value is not a string/number (never for a real token).
fn fmt_token(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => DASH.to_string(),
    }
}

/// Format the measured `decisions.status_present` object — a `{status_name: count}` map of
/// which decision-status records the country currently has (live country_containers fold).
/// Renders the present status name(s) (e.g. "active_timed_decision"); a measured-empty `{}`
/// renders "none". Only ever called on a `Measured` value, so it never fabricates.
fn fmt_status_present(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(m) if m.is_empty() => "none".to_string(),
        serde_json::Value::Object(m) => {
            let mut keys: Vec<&str> = m.keys().map(|k| k.as_str()).collect();
            keys.sort();
            keys.join(", ")
        }
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => DASH.to_string(),
    }
}

/// The grid gutter (scrollbar reservation), shared by all three row factories.
const GUTTER: f32 = 14.0;

/// Find a laid-out node's rect by name (depth-first), non-degenerate only.
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

/// Find a TEMPLATE window in a parsed `.gui` file by name (the row blueprint).
fn find_template(file: &GuiFile, name: &str) -> Option<Node> {
    let raw = file
        .windows
        .iter()
        .find(|w| w.name.as_deref() == Some(name))?;
    let manifest = assets::SpriteManifest {
        sprites: std::collections::HashMap::new(),
    };
    Some(Node::from_raw(raw, &manifest, &Loc::new()))
}

fn empty_gui() -> GuiFile {
    GuiFile::parse(r#"{"view":"empty","windows":[]}"#).expect("empty gui parses")
}

/// One resolved cell of a row: the target element name + the display atom bound into it.
/// The atom carries the no-fabrication glyph for absent/null/blocked fields.
#[derive(Clone)]
struct Cell {
    element: String,
    atom: DisplayAtom,
}

/// One instantiated data row: the element→atom cells for its text fields. Custom-drawn by
/// the view's `Drawable` through the template; recorded in the digest so a scrub is exact.
/// `gaps` holds LOGICAL fields with no backing `.gui` node (e.g. the `decision_status`
/// flag) — recorded for honesty + the digest, never bound to a node. Each gap carries its
/// `(label, atom)` so the digest reflects a MEASURED value when one is now available (and
/// "—" when still absent), preserving no-fabrication.
#[derive(Clone)]
struct DataRow {
    key: Option<String>,
    cells: Vec<Cell>,
    gaps: Vec<(String, DisplayAtom)>,
}

impl DataRow {
    fn new() -> DataRow {
        DataRow {
            key: None,
            cells: Vec::new(),
            gaps: Vec::new(),
        }
    }
    fn with_key(mut self, key: impl Into<String>) -> DataRow {
        self.key = Some(key.into());
        self
    }
    /// Bind `element` to `cap` formatted by `fmt` at `fidelity`, recording the atom into
    /// the report. Absent/Null render "—" automatically (no fabricated value).
    fn bind(
        &mut self,
        report: &mut FidelityReport,
        element: &str,
        cap: Captured<&serde_json::Value>,
        fidelity: Fidelity,
        fmt: impl FnOnce(&serde_json::Value) -> String,
    ) {
        let atom = cap.display_with_fidelity(fidelity, fmt);
        report.record(&atom);
        self.cells.push(Cell {
            element: element.to_string(),
            atom,
        });
    }
    /// Bind `element` to an explicit `unavailable` badge ("—") — a runtime-computed or
    /// live-resisted field we never fabricate. Recorded as unavailable in the report.
    /// `element` MUST name a real `.gui` node (verified by `every_bound_element_name_exists`).
    fn badge_unavailable(&mut self, report: &mut FidelityReport, element: &str) {
        let atom =
            DisplayAtom::unavailable_because(crate::host::context::UnavailableReason::Blocked);
        report.record(&atom);
        self.cells.push(Cell {
            element: element.to_string(),
            atom,
        });
    }

    /// Bind a LOGICAL field that has NO backing `.gui` element to a real captured value:
    /// a MEASURED value is tallied measured (so the section's fidelity reflects it), an
    /// Absent/Null value badges "—". It is NOT pushed as a targetable cell (no `.gui`
    /// node), only recorded into the fidelity report + the row's `gaps` (for the digest),
    /// so it never silently no-ops a `set_element_text` against a non-existent node. Used
    /// to surface the now-recorded `countries.<TAG>.decisions.status_present` flag.
    fn bind_gap(
        &mut self,
        report: &mut FidelityReport,
        label: &str,
        cap: Captured<&serde_json::Value>,
        fidelity: Fidelity,
        fmt: impl FnOnce(&serde_json::Value) -> String,
    ) {
        let atom = cap.display_with_fidelity(fidelity, fmt);
        report.record(&atom);
        self.gaps.push((label.to_string(), atom));
    }
}

/// A digest line for one data row (stable, comparable across a scrub).
fn row_digest(rows: &[DataRow]) -> String {
    let mut s = String::new();
    for (i, r) in rows.iter().enumerate() {
        s.push_str(&format!("r{i}["));
        if let Some(key) = &r.key {
            s.push_str(&format!("key={key};"));
        }
        for c in &r.cells {
            let badge = match c.atom.fidelity() {
                Fidelity::Exact => "E",
                Fidelity::Derived => "D",
                Fidelity::Unavailable => "U",
            };
            s.push_str(&format!("{}={}({badge});", c.element, c.atom.as_str()));
        }
        for (label, atom) in &r.gaps {
            // a gap reflects its MEASURED value when now available, else "—". The `(gap)`
            // marker is retained (no backing `.gui` node) so the digest still distinguishes a
            // logical gap from a real cell, but the value is the honest measured/absent atom.
            s.push_str(&format!("{label}={}(gap);", atom.as_str()));
        }
        s.push(']');
    }
    s
}

fn selected_row_digest(rows: &[DataRow], key: Option<&str>) -> String {
    match key.and_then(|selected| rows.iter().find(|row| row.key.as_deref() == Some(selected))) {
        Some(row) => row_digest(std::slice::from_ref(row)),
        None => "none".to_string(),
    }
}

fn row_key_token(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The shared per-row custom draw: instantiate the template into the viewport gridbox and
/// stamp each row's cell atoms into the named children, then draw the rows + scrollbar.
/// (Mirrors `state_panel::render_buildings_grid` — the gridbox plumbing is reused, not
/// re-implemented.)
fn render_rows(
    fd: &mut FrameDraw,
    panel: &[Node],
    file: &GuiFile,
    grid_name: &str,
    template_name: &str,
    rows: &[DataRow],
) {
    render_rows_selected(fd, panel, file, grid_name, template_name, rows, None);
}

fn render_rows_selected(
    fd: &mut FrameDraw,
    panel: &[Node],
    file: &GuiFile,
    grid_name: &str,
    template_name: &str,
    rows: &[DataRow],
    selected_key: Option<&str>,
) {
    let viewport = match find_rect(panel, grid_name) {
        Some(r) => r,
        None => return,
    };
    render_rows_selected_in_viewport(
        fd,
        file,
        grid_name,
        template_name,
        rows,
        viewport,
        0.0,
        selected_key,
    );
}

fn render_rows_selected_in_viewport(
    fd: &mut FrameDraw,
    file: &GuiFile,
    grid_name: &str,
    template_name: &str,
    rows: &[DataRow],
    viewport: Rect,
    scroll_y: f32,
    selected_key: Option<&str>,
) {
    let template = match find_template(file, template_name) {
        Some(t) => t,
        None => return,
    };
    if rows.is_empty() {
        return;
    }
    let manifest = &fd.stage.manifest;
    let spec = if grid_name == "decision_grid" && template_name == "timed_decision_item" {
        decision_grid_spec(viewport)
    } else {
        GridSpec::from_gridbox(&stub_grid(grid_name, viewport), &template, GUTTER)
    };
    let content = ManifestContent {
        manifest,
        measure: |_t, _f| None,
    };
    let mut inst = gridbox::instantiate(&template, rows.len(), viewport, &spec, scroll_y, &content);
    for (i, row) in inst.rows.iter_mut().enumerate() {
        for cell in &rows[i].cells {
            bind::apply::set_element_text(row, &cell.element, cell.atom.as_str());
        }
    }
    let mut stats = DrawStats::default();
    let selected_marker = if selected_key.is_some() {
        Some(widgets::common::white_texture(fd.renderer))
    } else {
        None
    };
    let textures: &mut TextureCache = fd.textures;
    let fonts: &mut FontCache = fd.fonts;
    for (row, data) in inst.rows.iter().zip(rows.iter()) {
        if selected_key.is_some()
            && data.key.as_deref() == selected_key
            && row.rect.y + row.rect.h > viewport.y
            && row.rect.y < viewport.y + viewport.h
        {
            if let Some(white) = selected_marker.as_ref() {
                widgets::common::fill_rect(
                    fd.renderer,
                    white,
                    row.rect.x + 1.0,
                    row.rect.y + 1.0,
                    row.rect.w - 2.0,
                    row.rect.h - 2.0,
                    [0.24, 0.34, 0.46, 0.70],
                    Some([viewport.x, viewport.y, viewport.w, viewport.h]),
                );
            }
        }
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

fn render_production_line_rows(
    fd: &mut FrameDraw,
    panel: &[Node],
    file: &GuiFile,
    grid_name: &str,
    rows: &[ProductionLineRow],
    scroll_y: f32,
    selected_key: Option<&str>,
) {
    let viewport = match find_rect(panel, grid_name) {
        Some(r) => r,
        None => return,
    };
    if rows.is_empty() {
        return;
    }
    let max_scroll = max_production_scroll_fallback(viewport, rows.len());
    let scroll_y = scroll_y.clamp(0.0, max_scroll);
    let manifest = &fd.stage.manifest;
    let content = ManifestContent {
        manifest,
        measure: |_t, _f| None,
    };
    let mut stats = DrawStats::default();
    let selected_marker = if selected_key.is_some() {
        Some(widgets::common::white_texture(fd.renderer))
    } else {
        None
    };
    let textures: &mut TextureCache = fd.textures;
    let fonts: &mut FontCache = fd.fonts;
    for (i, line) in rows.iter().enumerate() {
        let row_y = viewport.y + i as f32 * PRODUCTION_ROW_H - scroll_y;
        if row_y + PRODUCTION_ROW_H <= viewport.y || row_y >= viewport.y + viewport.h {
            continue;
        }
        let template = match find_template(file, line.kind.template_name()) {
            Some(t) => t,
            None => continue,
        };
        let mut row = template.clone();
        row.pos = (0.0, 0.0);
        let cell = Rect {
            x: viewport.x,
            y: row_y,
            w: (viewport.w - GUTTER).max(1.0),
            h: PRODUCTION_ROW_H,
        };
        layout(&mut row, cell, Some(viewport), &content);
        for cell in &line.row.cells {
            bind::apply::set_element_text(&mut row, &cell.element, cell.atom.as_str());
        }
        if selected_key.is_some()
            && line.row.key.as_deref() == selected_key
            && row.rect.y + row.rect.h > viewport.y
            && row.rect.y < viewport.y + viewport.h
        {
            if let Some(white) = selected_marker.as_ref() {
                widgets::common::fill_rect(
                    fd.renderer,
                    white,
                    row.rect.x + 1.0,
                    row.rect.y + 1.0,
                    row.rect.w - 2.0,
                    row.rect.h - 2.0,
                    [0.24, 0.34, 0.46, 0.70],
                    Some([viewport.x, viewport.y, viewport.w, viewport.h]),
                );
            }
        }
        gui_runtime::draw::draw_window(fd.renderer, &row, textures, fonts, &mut stats);
    }
    draw_grid_scrollbar(fd, viewport, rows.len() as f32 * PRODUCTION_ROW_H, scroll_y);
}

fn draw_grid_scrollbar(fd: &mut FrameDraw, viewport: Rect, content_height: f32, scroll_y: f32) {
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport.h, content_height);
    if max_scroll <= 0.0 {
        return;
    }
    let track = Rect {
        x: viewport.x + viewport.w - GUTTER,
        y: viewport.y,
        w: GUTTER,
        h: viewport.h,
    };
    let visible_frac = (viewport.h / content_height).clamp(0.05, 1.0);
    let thumb_h = (track.h * visible_frac).max(16.0).min(track.h);
    let scroll_frac = if max_scroll > 0.0 {
        scroll_y.clamp(0.0, max_scroll) / max_scroll
    } else {
        0.0
    };
    let thumb_y = track.y + (track.h - thumb_h).max(0.0) * scroll_frac.clamp(0.0, 1.0);
    let thumb = Rect {
        x: track.x,
        y: thumb_y,
        w: GUTTER,
        h: thumb_h,
    };
    if let Some(t) = fd.textures.get(fd.renderer, "GFX_scrollbar_vertical_bg") {
        let border = fd
            .stage
            .manifest
            .get("GFX_scrollbar_vertical_bg")
            .and_then(|e| e.border_size)
            .map(|b| [b[0] as f32, b[1] as f32])
            .unwrap_or([0.0, 52.0]);
        fd.renderer
            .draw_nine_slice(t, track.x, track.y, track.w, track.h, border);
    }
    if let Some(t) = fd.textures.get(fd.renderer, "GFX_generic_box_96") {
        fd.renderer
            .draw_sprite(t, thumb.x, thumb.y, thumb.w, thumb.h);
    }
}

fn stub_grid(name: &str, rect: Rect) -> Node {
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

fn decision_grid_spec(viewport: Rect) -> GridSpec {
    GridSpec {
        cell_w: (viewport.w - GUTTER).max(1.0),
        cell_h: DECISION_ROW_H,
        columns: 1,
        flow: gridbox::GridFlow::RowMajor,
        scrollbar_w: GUTTER,
    }
}

/// Set the bound summary atoms into the ROOT scene tree + lay it out. Shared by all three
/// views' `bind`. `placement` positions the (otherwise full-screen) ROOT window.
fn build_root(ctx: &BindContext, file: &GuiFile, root_name: &str, summary: &[Cell]) -> Vec<Node> {
    let stage = ctx.stage;
    let cm = ManifestContent {
        manifest: &stage.manifest,
        measure: |_t, _f| None,
    };
    let mut roots: Vec<Node> = build_scene(file, &stage.manifest, &stage.loc)
        .into_iter()
        .filter(|w| w.name == root_name)
        .collect();
    for cell in summary {
        for root in roots.iter_mut() {
            bind::apply::set_element_text(root, &cell.element, cell.atom.as_str());
        }
    }
    for root in roots.iter_mut() {
        layout(root, ctx.screen, None, &cm);
    }
    roots
}

/// Read an array field under the observed country as a slice, preserving the trichotomy:
/// returns `(Some(slice), Measured)` when present+array, `(None, state)` otherwise so the
/// caller records a SECTION-level fidelity (absent-array ≠ empty-array, Codex Q4).
fn country_array<'a>(
    ctx: &'a BindContext,
    tag: &str,
    dotted: &str,
) -> (Option<&'a [serde_json::Value]>, SectionState) {
    array_from_capture(ctx.country_field(tag, dotted))
}

fn array_from_capture(
    cap: Captured<&serde_json::Value>,
) -> (Option<&[serde_json::Value]>, SectionState) {
    match cap {
        Captured::Measured(v) => match v.as_array() {
            Some(a) => (Some(a.as_slice()), SectionState::Present),
            None => (None, SectionState::Degraded), // present but not an array
        },
        Captured::Null => (None, SectionState::Null),
        Captured::Absent => (None, SectionState::Absent),
    }
}

/// Why a row section has no rows — so "no data" (live channel absent) is never conflated
/// with "empty list" (measured, genuinely zero rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionState {
    /// Present and an array (rows may still be 0 = a measured empty list).
    Present,
    /// The whole array key is absent (the live channel is not folded here).
    Absent,
    /// Present but JSON null.
    Null,
    /// Present but not an array shape (degraded data).
    Degraded,
}

impl SectionState {
    /// Record the section's honesty: a Present empty/non-empty list is a measured fact;
    /// an Absent/Null/Degraded section is `unavailable` (one badge so the per-view report
    /// honestly flags "this list could not be obtained", never silently "empty").
    fn record(self, report: &mut FidelityReport) {
        match self {
            SectionState::Present => report.exact += 1,
            _ => report.unavailable += 1,
        }
    }
    fn tag(self) -> &'static str {
        match self {
            SectionState::Present => "present",
            SectionState::Absent => "absent",
            SectionState::Null => "null",
            SectionState::Degraded => "degraded",
        }
    }
}

// =====================================================================================
// VIEW 1 — PRODUCTION
// =====================================================================================

const PRODUCTION_VIEW_ID: ViewId = ViewId("production.production");
const PRODUCTION_FILE: &str = "production.json";
const PRODUCTION_ROOT: &str = "countryproductionlineview";
const PRODUCTION_ROW_H: f32 = 111.0;
const PRODUCTION_LINE_CLOSE_BUTTON_SIZE: f32 = 24.0;
const PRODUCTION_FACTORY_BUTTON_SIZE: f32 = 24.0;
const PRODUCTION_PRIORITY_BUTTON_SIZE: f32 = 24.0;
const PRODUCTION_AMOUNT_BUTTON_SIZE: f32 = 24.0;
const PRODUCTION_ADD_LINE_BUTTON_W: f32 = 81.0;
const PRODUCTION_ADD_LINE_BUTTON_H: f32 = 41.0;
const PRODUCTION_PRIMARY_FILTER_BUTTON_W: f32 = 45.0;
const PRODUCTION_PRIMARY_FILTER_BUTTON_H: f32 = 32.0;
const PRODUCTION_STATUS_FILTER_BUTTON_W: f32 = 45.0;
const PRODUCTION_STATUS_FILTER_BUTTON_H: f32 = 32.0;
const PRODUCTION_REFIT_FILTER_BUTTON_H: f32 = 35.0;
const PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE: f32 = 24.0;
const PRODUCTION_EQUIPMENT_BUTTON_W: f32 = 96.0;
const PRODUCTION_EQUIPMENT_BUTTON_H: f32 = 64.0;

pub struct ProductionView {
    selected_line_key: Option<String>,
    line_keys: Vec<Option<String>>,
    line_kinds: Vec<ProductionLineKind>,
    primary_filters: ProductionPrimaryFilters,
    line_scroll_y: f32,
    line_scroll_drag: Option<ProductionScrollDrag>,
    screen: Rect,
}

impl ProductionView {
    pub fn new() -> ProductionView {
        ProductionView {
            selected_line_key: None,
            line_keys: Vec::new(),
            line_kinds: Vec::new(),
            primary_filters: ProductionPrimaryFilters::default(),
            line_scroll_y: 0.0,
            line_scroll_drag: None,
            screen: default_production_screen(),
        }
    }
}

impl Default for ProductionView {
    fn default() -> Self {
        ProductionView::new()
    }
}

fn default_production_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn production_grid_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 592.0,
        y: screen.y + 336.0,
        w: 536.0,
        h: (screen.h - 336.0).max(0.0),
    }
}

fn production_panel_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 606.0,
        y: screen.y + 78.0,
        w: 550.0,
        h: (screen.h - 78.0).max(0.0),
    }
}

fn production_add_line_button_rects(screen: Rect) -> [Rect; 5] {
    let panel = production_panel_fallback(screen);
    let y = panel.y + 30.0 + 14.0 + 122.0;
    [
        Rect {
            x: panel.x + 25.0,
            y,
            w: PRODUCTION_ADD_LINE_BUTTON_W,
            h: PRODUCTION_ADD_LINE_BUTTON_H,
        },
        Rect {
            x: panel.x + 105.0,
            y,
            w: PRODUCTION_ADD_LINE_BUTTON_W,
            h: PRODUCTION_ADD_LINE_BUTTON_H,
        },
        Rect {
            x: panel.x + 185.0,
            y,
            w: PRODUCTION_ADD_LINE_BUTTON_W,
            h: PRODUCTION_ADD_LINE_BUTTON_H,
        },
        Rect {
            x: panel.x + 350.0,
            y,
            w: PRODUCTION_ADD_LINE_BUTTON_W,
            h: PRODUCTION_ADD_LINE_BUTTON_H,
        },
        Rect {
            x: panel.x + 430.0,
            y,
            w: PRODUCTION_ADD_LINE_BUTTON_W,
            h: PRODUCTION_ADD_LINE_BUTTON_H,
        },
    ]
}

fn production_add_line_button_hit(screen: Rect, x: f32, y: f32) -> bool {
    production_add_line_button_rects(screen)
        .iter()
        .any(|rect| rect.contains(x, y))
}

fn production_primary_filter_button_rects(screen: Rect) -> [(ProductionPrimaryFilter, Rect); 6] {
    let panel = production_panel_fallback(screen);
    let y = panel.y + 30.0 + 14.0 + 179.0;
    [
        (
            ProductionPrimaryFilter::Equipment,
            Rect {
                x: panel.x + 13.0,
                y,
                w: PRODUCTION_PRIMARY_FILTER_BUTTON_W,
                h: PRODUCTION_PRIMARY_FILTER_BUTTON_H,
            },
        ),
        (
            ProductionPrimaryFilter::Armor,
            Rect {
                x: panel.x + 98.0,
                y,
                w: PRODUCTION_PRIMARY_FILTER_BUTTON_W,
                h: PRODUCTION_PRIMARY_FILTER_BUTTON_H,
            },
        ),
        (
            ProductionPrimaryFilter::Air,
            Rect {
                x: panel.x + 183.0,
                y,
                w: PRODUCTION_PRIMARY_FILTER_BUTTON_W,
                h: PRODUCTION_PRIMARY_FILTER_BUTTON_H,
            },
        ),
        (
            ProductionPrimaryFilter::CapitalShips,
            Rect {
                x: panel.x + 268.0,
                y,
                w: PRODUCTION_PRIMARY_FILTER_BUTTON_W,
                h: PRODUCTION_PRIMARY_FILTER_BUTTON_H,
            },
        ),
        (
            ProductionPrimaryFilter::ScreenShips,
            Rect {
                x: panel.x + 353.0,
                y,
                w: PRODUCTION_PRIMARY_FILTER_BUTTON_W,
                h: PRODUCTION_PRIMARY_FILTER_BUTTON_H,
            },
        ),
        (
            ProductionPrimaryFilter::OtherShips,
            Rect {
                x: panel.x + 439.0,
                y,
                w: PRODUCTION_PRIMARY_FILTER_BUTTON_W,
                h: PRODUCTION_PRIMARY_FILTER_BUTTON_H,
            },
        ),
    ]
}

fn production_primary_filter_button_hit(
    screen: Rect,
    x: f32,
    y: f32,
) -> Option<ProductionPrimaryFilter> {
    production_primary_filter_button_rects(screen)
        .into_iter()
        .find_map(|(filter, rect)| rect.contains(x, y).then_some(filter))
}

fn production_status_filter_button_rects(screen: Rect) -> [(ProductionStatusFilter, Rect); 6] {
    let panel = production_panel_fallback(screen);
    let y = panel.y + 30.0 + 14.0 + 211.0;
    [
        (
            ProductionStatusFilter::Resources,
            Rect {
                x: panel.x + 98.0,
                y,
                w: PRODUCTION_STATUS_FILTER_BUTTON_W,
                h: PRODUCTION_STATUS_FILTER_BUTTON_H,
            },
        ),
        (
            ProductionStatusFilter::Outdated,
            Rect {
                x: panel.x + 183.0,
                y,
                w: PRODUCTION_STATUS_FILTER_BUTTON_W,
                h: PRODUCTION_STATUS_FILTER_BUTTON_H,
            },
        ),
        (
            ProductionStatusFilter::Stockpiling,
            Rect {
                x: panel.x + 268.0,
                y,
                w: PRODUCTION_STATUS_FILTER_BUTTON_W,
                h: PRODUCTION_STATUS_FILTER_BUTTON_H,
            },
        ),
        (
            ProductionStatusFilter::Reinforcing,
            Rect {
                x: panel.x + 353.0,
                y,
                w: PRODUCTION_STATUS_FILTER_BUTTON_W,
                h: PRODUCTION_STATUS_FILTER_BUTTON_H,
            },
        ),
        (
            ProductionStatusFilter::Upgrading,
            Rect {
                x: panel.x + 440.0,
                y,
                w: PRODUCTION_STATUS_FILTER_BUTTON_W,
                h: PRODUCTION_STATUS_FILTER_BUTTON_H,
            },
        ),
        (
            ProductionStatusFilter::Refitting,
            Rect {
                x: panel.x + 14.0,
                y: panel.y + 30.0 + 14.0 + 208.0,
                w: PRODUCTION_STATUS_FILTER_BUTTON_W,
                h: PRODUCTION_REFIT_FILTER_BUTTON_H,
            },
        ),
    ]
}

fn production_status_filter_button_hit(screen: Rect, x: f32, y: f32) -> bool {
    production_status_filter_button_rects(screen)
        .into_iter()
        .any(|(_, rect)| rect.contains(x, y))
}

fn production_line_close_button_rect(viewport: Rect, row_y: f32) -> Rect {
    Rect {
        x: viewport.x + 481.0,
        y: row_y + 2.0,
        w: PRODUCTION_LINE_CLOSE_BUTTON_SIZE,
        h: PRODUCTION_LINE_CLOSE_BUTTON_SIZE,
    }
}

fn production_line_close_button_hit(
    viewport: Rect,
    row_count: usize,
    scroll_y: f32,
    x: f32,
    y: f32,
) -> bool {
    if x < viewport.x
        || x > viewport.x + viewport.w
        || y < viewport.y
        || y > viewport.y + viewport.h
    {
        return false;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / PRODUCTION_ROW_H).floor() as usize;
    if idx >= row_count {
        return false;
    }
    let row_y = viewport.y + idx as f32 * PRODUCTION_ROW_H - scroll_y.max(0.0);
    production_line_close_button_rect(viewport, row_y).contains(x, y)
}

fn production_factory_count_button_rects(viewport: Rect, row_y: f32) -> [Rect; 2] {
    [
        Rect {
            x: viewport.x + 379.0,
            y: row_y + 2.0,
            w: PRODUCTION_FACTORY_BUTTON_SIZE,
            h: PRODUCTION_FACTORY_BUTTON_SIZE,
        },
        Rect {
            x: viewport.x + 453.0,
            y: row_y + 2.0,
            w: PRODUCTION_FACTORY_BUTTON_SIZE,
            h: PRODUCTION_FACTORY_BUTTON_SIZE,
        },
    ]
}

fn production_factory_count_button_hit(
    viewport: Rect,
    row_count: usize,
    scroll_y: f32,
    x: f32,
    y: f32,
) -> bool {
    if x < viewport.x
        || x > viewport.x + viewport.w
        || y < viewport.y
        || y > viewport.y + viewport.h
    {
        return false;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / PRODUCTION_ROW_H).floor() as usize;
    if idx >= row_count {
        return false;
    }
    let row_y = viewport.y + idx as f32 * PRODUCTION_ROW_H - scroll_y.max(0.0);
    production_factory_count_button_rects(viewport, row_y)
        .iter()
        .any(|rect| rect.contains(x, y))
}

fn production_priority_button_rects(viewport: Rect, row_y: f32) -> [Rect; 2] {
    [
        Rect {
            x: viewport.x + 4.0,
            y: row_y + 48.0,
            w: PRODUCTION_PRIORITY_BUTTON_SIZE,
            h: PRODUCTION_PRIORITY_BUTTON_SIZE,
        },
        Rect {
            x: viewport.x + 4.0,
            y: row_y + 75.0,
            w: PRODUCTION_PRIORITY_BUTTON_SIZE,
            h: PRODUCTION_PRIORITY_BUTTON_SIZE,
        },
    ]
}

fn production_amount_button_rects(
    viewport: Rect,
    row_y: f32,
    kind: ProductionLineKind,
) -> [Rect; 2] {
    match kind {
        ProductionLineKind::Military => [
            Rect {
                x: viewport.x + 278.0,
                y: row_y + 30.0,
                w: PRODUCTION_AMOUNT_BUTTON_SIZE,
                h: PRODUCTION_AMOUNT_BUTTON_SIZE,
            },
            Rect {
                x: viewport.x + 278.0,
                y: row_y + 68.0,
                w: PRODUCTION_AMOUNT_BUTTON_SIZE,
                h: PRODUCTION_AMOUNT_BUTTON_SIZE,
            },
        ],
        ProductionLineKind::Naval => [
            Rect {
                x: viewport.x + 282.0,
                y: row_y + 26.0,
                w: PRODUCTION_AMOUNT_BUTTON_SIZE,
                h: PRODUCTION_AMOUNT_BUTTON_SIZE,
            },
            Rect {
                x: viewport.x + 282.0,
                y: row_y + 74.0,
                w: PRODUCTION_AMOUNT_BUTTON_SIZE,
                h: PRODUCTION_AMOUNT_BUTTON_SIZE,
            },
        ],
    }
}

fn production_priority_amount_button_hit(
    viewport: Rect,
    row_kinds: &[ProductionLineKind],
    scroll_y: f32,
    x: f32,
    y: f32,
) -> bool {
    if x < viewport.x
        || x > viewport.x + viewport.w
        || y < viewport.y
        || y > viewport.y + viewport.h
    {
        return false;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / PRODUCTION_ROW_H).floor() as usize;
    let Some(kind) = row_kinds.get(idx).copied() else {
        return false;
    };
    let row_y = viewport.y + idx as f32 * PRODUCTION_ROW_H - scroll_y.max(0.0);
    production_priority_button_rects(viewport, row_y)
        .into_iter()
        .chain(production_amount_button_rects(viewport, row_y, kind))
        .any(|rect| rect.contains(x, y))
}

fn production_line_mutation_control_rects(
    viewport: Rect,
    row_y: f32,
    kind: ProductionLineKind,
) -> Vec<Rect> {
    let equipment_x = match kind {
        ProductionLineKind::Military => 1.0,
        ProductionLineKind::Naval => 0.0,
    };
    let mut rects = vec![
        Rect {
            x: viewport.x + 195.0,
            y: row_y + 1.0,
            w: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
            h: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
        },
        Rect {
            x: viewport.x + 195.0,
            y: row_y + 1.0,
            w: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
            h: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
        },
        Rect {
            x: viewport.x + equipment_x,
            y: row_y + 27.0,
            w: PRODUCTION_EQUIPMENT_BUTTON_W,
            h: PRODUCTION_EQUIPMENT_BUTTON_H,
        },
        Rect {
            x: viewport.x + 13.0,
            y: row_y + 8.0,
            w: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
            h: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
        },
    ];
    match kind {
        ProductionLineKind::Military => rects.push(Rect {
            x: viewport.x + 30.0,
            y: row_y + 75.0,
            w: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
            h: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
        }),
        ProductionLineKind::Naval => {
            rects.push(Rect {
                x: viewport.x + 245.0,
                y: row_y + 50.0,
                w: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
                h: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
            });
            rects.push(Rect {
                x: viewport.x + 230.0,
                y: row_y - 1.0,
                w: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
                h: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
            });
            rects.push(Rect {
                x: viewport.x + 315.0,
                y: row_y + 81.0,
                w: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
                h: PRODUCTION_LINE_SMALL_MUTATION_BUTTON_SIZE,
            });
        }
    }
    rects
}

fn production_line_mutation_control_hit(
    viewport: Rect,
    row_kinds: &[ProductionLineKind],
    scroll_y: f32,
    x: f32,
    y: f32,
) -> bool {
    if x < viewport.x
        || x > viewport.x + viewport.w
        || y < viewport.y
        || y > viewport.y + viewport.h
    {
        return false;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / PRODUCTION_ROW_H).floor() as usize;
    let Some(kind) = row_kinds.get(idx).copied() else {
        return false;
    };
    let row_y = viewport.y + idx as f32 * PRODUCTION_ROW_H - scroll_y.max(0.0);
    production_line_mutation_control_rects(viewport, row_y, kind)
        .into_iter()
        .any(|rect| rect.contains(x, y))
}

fn production_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProductionLineKind {
    Military,
    Naval,
}

impl ProductionLineKind {
    fn key_part(self) -> &'static str {
        match self {
            ProductionLineKind::Military => "military",
            ProductionLineKind::Naval => "naval",
        }
    }
    fn template_name(self) -> &'static str {
        match self {
            ProductionLineKind::Military => "production_military_line_entry",
            ProductionLineKind::Naval => "production_naval_line_entry",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProductionPrimaryFilter {
    Equipment,
    Armor,
    Air,
    CapitalShips,
    ScreenShips,
    OtherShips,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProductionStatusFilter {
    Resources,
    Outdated,
    Stockpiling,
    Reinforcing,
    Upgrading,
    Refitting,
}

impl ProductionPrimaryFilter {
    fn key(self) -> &'static str {
        match self {
            ProductionPrimaryFilter::Equipment => "equipment",
            ProductionPrimaryFilter::Armor => "armor",
            ProductionPrimaryFilter::Air => "air",
            ProductionPrimaryFilter::CapitalShips => "capital_ships",
            ProductionPrimaryFilter::ScreenShips => "screen_ships",
            ProductionPrimaryFilter::OtherShips => "other_ships",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProductionPrimaryFilters {
    equipment: bool,
    armor: bool,
    air: bool,
    capital_ships: bool,
    screen_ships: bool,
    other_ships: bool,
}

impl Default for ProductionPrimaryFilters {
    fn default() -> Self {
        ProductionPrimaryFilters {
            equipment: true,
            armor: true,
            air: true,
            capital_ships: true,
            screen_ships: true,
            other_ships: true,
        }
    }
}

impl ProductionPrimaryFilters {
    fn is_active(self, filter: ProductionPrimaryFilter) -> bool {
        match filter {
            ProductionPrimaryFilter::Equipment => self.equipment,
            ProductionPrimaryFilter::Armor => self.armor,
            ProductionPrimaryFilter::Air => self.air,
            ProductionPrimaryFilter::CapitalShips => self.capital_ships,
            ProductionPrimaryFilter::ScreenShips => self.screen_ships,
            ProductionPrimaryFilter::OtherShips => self.other_ships,
        }
    }

    fn toggle(&mut self, filter: ProductionPrimaryFilter) {
        match filter {
            ProductionPrimaryFilter::Equipment => self.equipment = !self.equipment,
            ProductionPrimaryFilter::Armor => self.armor = !self.armor,
            ProductionPrimaryFilter::Air => self.air = !self.air,
            ProductionPrimaryFilter::CapitalShips => self.capital_ships = !self.capital_ships,
            ProductionPrimaryFilter::ScreenShips => self.screen_ships = !self.screen_ships,
            ProductionPrimaryFilter::OtherShips => self.other_ships = !self.other_ships,
        }
    }

    fn allows(self, category: Option<ProductionPrimaryFilter>) -> bool {
        category
            .map(|category| self.is_active(category))
            .unwrap_or(true)
    }

    fn digest(self) -> String {
        [
            (ProductionPrimaryFilter::Equipment, self.equipment),
            (ProductionPrimaryFilter::Armor, self.armor),
            (ProductionPrimaryFilter::Air, self.air),
            (ProductionPrimaryFilter::CapitalShips, self.capital_ships),
            (ProductionPrimaryFilter::ScreenShips, self.screen_ships),
            (ProductionPrimaryFilter::OtherShips, self.other_ships),
        ]
        .into_iter()
        .map(|(filter, active)| format!("{}={}", filter.key(), if active { 1 } else { 0 }))
        .collect::<Vec<_>>()
        .join(",")
    }
}

#[derive(Clone)]
struct ProductionLineRow {
    kind: ProductionLineKind,
    primary_filter: Option<ProductionPrimaryFilter>,
    row: DataRow,
}

#[derive(Debug, Default)]
struct EquipmentTypeCatalog {
    entries: HashMap<String, EquipmentTypeEntry>,
}

#[derive(Debug, Default)]
struct EquipmentTypeEntry {
    parent: Option<String>,
    types: Vec<String>,
    source_file: Option<String>,
}

static EQUIPMENT_TYPE_CATALOG: OnceLock<EquipmentTypeCatalog> = OnceLock::new();

impl EquipmentTypeCatalog {
    fn get() -> &'static EquipmentTypeCatalog {
        EQUIPMENT_TYPE_CATALOG.get_or_init(EquipmentTypeCatalog::load)
    }

    fn load() -> EquipmentTypeCatalog {
        Self::catalog_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|txt| Self::parse(&txt))
            .unwrap_or_else(|| EquipmentTypeCatalog {
                entries: HashMap::new(),
            })
    }

    fn catalog_path() -> Option<PathBuf> {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let replay_native = crate_dir.parent()?;
        let repo = replay_native.parent()?;
        Some(repo.join("game_defs").join("out").join("catalogs.json"))
    }

    fn parse(txt: &str) -> Option<EquipmentTypeCatalog> {
        let v: serde_json::Value = serde_json::from_str(txt).ok()?;
        let entries = v
            .get("catalogs")?
            .get("equipment_archetypes")?
            .as_object()?
            .iter()
            .map(|(token, def)| {
                (
                    token.clone(),
                    EquipmentTypeEntry {
                        parent: catalog_scalar(def.get("archetype")),
                        types: catalog_type_tokens(def.get("type")),
                        source_file: catalog_scalar(def.get("source_file")),
                    },
                )
            })
            .collect();
        Some(EquipmentTypeCatalog { entries })
    }

    fn type_tokens(&self, token: &str) -> Option<&[String]> {
        let mut current = Some(token);
        for _ in 0..8 {
            let entry = self.entries.get(current?)?;
            if !entry.types.is_empty() {
                return Some(entry.types.as_slice());
            }
            current = entry.parent.as_deref();
        }
        None
    }

    fn primary_filter(
        &self,
        token: &str,
        kind: ProductionLineKind,
    ) -> Option<ProductionPrimaryFilter> {
        match kind {
            ProductionLineKind::Military => {
                let Some(types) = self.type_tokens(token) else {
                    return self
                        .has_plane_airframe_source(token)
                        .then_some(ProductionPrimaryFilter::Air);
                };
                if types.iter().any(|t| production_type_is_air(t)) {
                    Some(ProductionPrimaryFilter::Air)
                } else if types.iter().any(|t| t == "armor") {
                    Some(ProductionPrimaryFilter::Armor)
                } else {
                    Some(ProductionPrimaryFilter::Equipment)
                }
            }
            ProductionLineKind::Naval => {
                let types = self.type_tokens(token)?;
                if types.iter().any(|t| t == "capital_ship" || t == "carrier") {
                    Some(ProductionPrimaryFilter::CapitalShips)
                } else if types.iter().any(|t| t == "screen_ship") {
                    Some(ProductionPrimaryFilter::ScreenShips)
                } else {
                    Some(ProductionPrimaryFilter::OtherShips)
                }
            }
        }
    }

    fn has_plane_airframe_source(&self, token: &str) -> bool {
        let mut current = Some(token);
        for _ in 0..8 {
            let Some(entry) = current.and_then(|token| self.entries.get(token)) else {
                return false;
            };
            if entry
                .source_file
                .as_deref()
                .map(|source| source.ends_with("plane_airframes.txt"))
                .unwrap_or(false)
            {
                return true;
            }
            current = entry.parent.as_deref();
        }
        false
    }
}

fn catalog_scalar(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn catalog_type_tokens(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::String(s)) if !s.is_empty() => vec![s.clone()],
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn production_type_is_air(token: &str) -> bool {
    matches!(
        token,
        "fighter"
            | "interceptor"
            | "cas"
            | "naval_bomber"
            | "tactical_bomber"
            | "strategic_bomber"
            | "scout_plane"
            | "transport"
            | "suicide"
    )
}

#[derive(Clone, Copy)]
struct ProductionLineWindow {
    scroll_y: f32,
    max_scroll: f32,
    first: usize,
    visible: usize,
}

#[derive(Clone, Copy)]
struct ProductionScrollDrag {
    pointer_offset_y: f32,
}

#[derive(Clone, Copy)]
struct ProductionScrollbarGeometry {
    track: Rect,
    thumb: Rect,
    max_scroll: f32,
}

fn max_production_scroll_fallback(viewport: Rect, row_count: usize) -> f32 {
    (row_count as f32 * PRODUCTION_ROW_H - viewport.h).max(0.0)
}

fn production_scrollbar_geometry(
    viewport: Rect,
    row_count: usize,
    scroll_y: f32,
) -> Option<ProductionScrollbarGeometry> {
    let content_h = row_count as f32 * PRODUCTION_ROW_H;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport.h, content_h);
    if max_scroll <= 0.0 {
        return None;
    }
    let track = Rect {
        x: viewport.x + viewport.w - GUTTER,
        y: viewport.y,
        w: GUTTER,
        h: viewport.h,
    };
    let visible_frac = (viewport.h / content_h).clamp(0.05, 1.0);
    let thumb_h = (track.h * visible_frac).max(16.0).min(track.h);
    let scroll_frac = if max_scroll > 0.0 {
        scroll_y.clamp(0.0, max_scroll) / max_scroll
    } else {
        0.0
    };
    let thumb_y = track.y + (track.h - thumb_h).max(0.0) * scroll_frac.clamp(0.0, 1.0);
    Some(ProductionScrollbarGeometry {
        track,
        thumb: Rect {
            x: track.x,
            y: thumb_y,
            w: GUTTER,
            h: thumb_h,
        },
        max_scroll,
    })
}

fn production_scroll_from_thumb_y(geom: ProductionScrollbarGeometry, thumb_y: f32) -> f32 {
    let travel = (geom.track.h - geom.thumb.h).max(0.0);
    if travel <= 0.0 || geom.max_scroll <= 0.0 {
        return 0.0;
    }
    let y = thumb_y.clamp(geom.track.y, geom.track.y + travel);
    ((y - geom.track.y) / travel) * geom.max_scroll
}

fn scroll_production_offset(current: f32, viewport: Rect, row_count: usize, dy: f32) -> f32 {
    if dy == 0.0 || !dy.is_finite() {
        return current;
    }
    let rows = dy.abs().ceil().max(1.0);
    let delta = rows * PRODUCTION_ROW_H;
    let next = if dy < 0.0 {
        current + delta
    } else {
        (current - delta).max(0.0)
    };
    next.min(max_production_scroll_fallback(viewport, row_count))
}

fn production_grid_window(
    roots: &[Node],
    file: &GuiFile,
    row_count: usize,
    requested_scroll: f32,
    fallback_viewport: Rect,
) -> ProductionLineWindow {
    let metrics = find_rect(roots, "item_grid")
        .and_then(|viewport| {
            find_template(file, "production_military_line_entry").map(|t| (viewport, t))
        })
        .map(|(viewport, template)| {
            let spec = GridSpec::from_gridbox(&stub_grid("item_grid", viewport), &template, GUTTER);
            (viewport.h, spec.cell_h.max(1.0), spec.columns.max(1))
        });
    let (viewport_h, cell_h, columns) = match metrics {
        Some((viewport_h, cell_h, columns)) => (viewport_h, cell_h, columns),
        None => (fallback_viewport.h, PRODUCTION_ROW_H, 1),
    };
    let grid_rows = row_count.div_ceil(columns);
    let content_h = grid_rows as f32 * cell_h;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport_h, content_h);
    let scroll_y = requested_scroll.clamp(0.0, max_scroll);
    let first_grid_row = (scroll_y / cell_h).floor() as usize;
    let visible_grid_rows = (viewport_h / cell_h).ceil().max(1.0) as usize + 1;
    ProductionLineWindow {
        scroll_y,
        max_scroll,
        first: first_grid_row.saturating_mul(columns),
        visible: visible_grid_rows.saturating_mul(columns),
    }
}

fn production_line_row_digest(rows: &[ProductionLineRow]) -> String {
    let mut s = String::new();
    for (i, r) in rows.iter().enumerate() {
        s.push_str(&format!("r{i}<{}>", r.kind.key_part()));
        s.push_str(&row_digest(std::slice::from_ref(&r.row)));
    }
    s
}

fn selected_production_line_digest(rows: &[ProductionLineRow], key: Option<&str>) -> String {
    match key.and_then(|selected| rows.iter().find(|r| r.row.key.as_deref() == Some(selected))) {
        Some(row) => production_line_row_digest(std::slice::from_ref(row)),
        None => "none".to_string(),
    }
}

fn visible_production_line_digest(
    rows: &[ProductionLineRow],
    window: ProductionLineWindow,
) -> String {
    let visible: Vec<ProductionLineRow> = rows
        .iter()
        .skip(window.first)
        .take(window.visible)
        .cloned()
        .collect();
    production_line_row_digest(&visible)
}

fn production_line_key(
    line: &serde_json::Value,
    kind: ProductionLineKind,
    save_channel: bool,
) -> Option<String> {
    let primary = if save_channel { "line_id" } else { "id" };
    match elem_field(line, primary) {
        Captured::Measured(v) => row_key_token(v).map(|id| {
            let source = if save_channel { "line" } else { "id" };
            format!("production:{}:{source}:{id}", kind.key_part())
        }),
        _ => None,
    }
}

fn suppress_duplicate_keys(keys: &mut [Option<String>]) {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for key in keys.iter().flatten() {
        *counts.entry(key.clone()).or_insert(0) += 1;
    }
    for key in keys.iter_mut() {
        let duplicate = match key.as_ref().and_then(|k| counts.get(k)) {
            Some(count) => *count > 1,
            None => false,
        };
        if duplicate {
            *key = None;
        }
    }
}

fn suppress_duplicate_row_keys(rows: &mut [DataRow]) {
    let mut keys: Vec<Option<String>> = rows.iter().map(|row| row.key.clone()).collect();
    suppress_duplicate_keys(&mut keys);
    for (row, key) in rows.iter_mut().zip(keys) {
        row.key = key;
    }
}

fn production_line_keys_from_arrays(
    live_military: Captured<&serde_json::Value>,
    live_naval: Captured<&serde_json::Value>,
    save_lines: Captured<&serde_json::Value>,
) -> Vec<Option<String>> {
    let (live_mil, _) = array_from_capture(live_military);
    let (live_nav, _) = array_from_capture(live_naval);
    let (save, _) = array_from_capture(save_lines);
    let mut keys = Vec::new();
    if let Some(arr) = live_mil {
        keys.extend(
            arr.iter()
                .map(|line| production_line_key(line, ProductionLineKind::Military, false)),
        );
    } else if let Some(arr) = save {
        keys.extend(
            arr.iter()
                .filter(|line| line.get("kind").and_then(|k| k.as_str()) == Some("military_lines"))
                .map(|line| production_line_key(line, ProductionLineKind::Military, true)),
        );
    }
    if let Some(arr) = live_nav {
        keys.extend(
            arr.iter()
                .map(|line| production_line_key(line, ProductionLineKind::Naval, false)),
        );
    } else if let Some(arr) = save {
        keys.extend(
            arr.iter()
                .filter(|line| line.get("kind").and_then(|k| k.as_str()) == Some("naval_lines"))
                .map(|line| production_line_key(line, ProductionLineKind::Naval, true)),
        );
    }
    suppress_duplicate_keys(&mut keys);
    keys
}

fn production_line_kinds_from_arrays(
    live_military: Captured<&serde_json::Value>,
    live_naval: Captured<&serde_json::Value>,
    save_lines: Captured<&serde_json::Value>,
) -> Vec<ProductionLineKind> {
    let (live_mil, _) = array_from_capture(live_military);
    let (live_nav, _) = array_from_capture(live_naval);
    let (save, _) = array_from_capture(save_lines);
    let mut kinds = Vec::new();
    if let Some(arr) = live_mil {
        kinds.extend(std::iter::repeat(ProductionLineKind::Military).take(arr.len()));
    } else if let Some(arr) = save {
        kinds.extend(
            arr.iter()
                .filter(|line| line.get("kind").and_then(|k| k.as_str()) == Some("military_lines"))
                .map(|_| ProductionLineKind::Military),
        );
    }
    if let Some(arr) = live_nav {
        kinds.extend(std::iter::repeat(ProductionLineKind::Naval).take(arr.len()));
    } else if let Some(arr) = save {
        kinds.extend(
            arr.iter()
                .filter(|line| line.get("kind").and_then(|k| k.as_str()) == Some("naval_lines"))
                .map(|_| ProductionLineKind::Naval),
        );
    }
    kinds
}

fn production_line_categories_from_arrays(
    live_military: Captured<&serde_json::Value>,
    live_naval: Captured<&serde_json::Value>,
    save_lines: Captured<&serde_json::Value>,
) -> Vec<Option<ProductionPrimaryFilter>> {
    let (live_mil, _) = array_from_capture(live_military);
    let (live_nav, _) = array_from_capture(live_naval);
    let (save, _) = array_from_capture(save_lines);
    let mut categories = Vec::new();
    if let Some(arr) = live_mil {
        categories.extend(arr.iter().map(|line| {
            production_line_primary_filter_with_save(line, ProductionLineKind::Military, save)
        }));
    } else if let Some(arr) = save {
        categories.extend(
            arr.iter()
                .filter(|line| line.get("kind").and_then(|k| k.as_str()) == Some("military_lines"))
                .map(|line| production_line_primary_filter(line, ProductionLineKind::Military)),
        );
    }
    if let Some(arr) = live_nav {
        categories.extend(arr.iter().map(|line| {
            production_line_primary_filter_with_save(line, ProductionLineKind::Naval, save)
        }));
    } else if let Some(arr) = save {
        categories.extend(
            arr.iter()
                .filter(|line| line.get("kind").and_then(|k| k.as_str()) == Some("naval_lines"))
                .map(|line| production_line_primary_filter(line, ProductionLineKind::Naval)),
        );
    }
    categories
}

fn production_line_primary_filter(
    line: &serde_json::Value,
    kind: ProductionLineKind,
) -> Option<ProductionPrimaryFilter> {
    production_line_archetype_token(line)
        .and_then(|token| EquipmentTypeCatalog::get().primary_filter(&token, kind))
}

fn production_line_primary_filter_with_save(
    line: &serde_json::Value,
    kind: ProductionLineKind,
    save_lines: Option<&[serde_json::Value]>,
) -> Option<ProductionPrimaryFilter> {
    if let Some(filter) = production_line_primary_filter(line, kind) {
        return Some(filter);
    }
    unique_save_line_for_live(line, kind, save_lines).and_then(|save| {
        production_line_archetype_token(save)
            .and_then(|token| EquipmentTypeCatalog::get().primary_filter(&token, kind))
    })
}

fn production_line_archetype_token(line: &serde_json::Value) -> Option<String> {
    line.get("archetype").and_then(row_key_token)
}

fn unique_save_line_for_live<'a>(
    live: &serde_json::Value,
    kind: ProductionLineKind,
    save_lines: Option<&'a [serde_json::Value]>,
) -> Option<&'a serde_json::Value> {
    let live_id = live.get("id").and_then(row_key_token)?;
    let save_kind = match kind {
        ProductionLineKind::Military => "military_lines",
        ProductionLineKind::Naval => "naval_lines",
    };
    let mut matches = save_lines?
        .iter()
        .filter(|line| line.get("kind").and_then(|k| k.as_str()) == Some(save_kind))
        .filter(|line| {
            line.get("line_id").and_then(row_key_token).as_deref() == Some(live_id.as_str())
        });
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

fn filter_production_line_metadata(
    keys: Vec<Option<String>>,
    kinds: Vec<ProductionLineKind>,
    categories: Vec<Option<ProductionPrimaryFilter>>,
    filters: ProductionPrimaryFilters,
) -> (Vec<Option<String>>, Vec<ProductionLineKind>) {
    let mut filtered_keys = Vec::new();
    let mut filtered_kinds = Vec::new();
    for (idx, (key, kind)) in keys.into_iter().zip(kinds).enumerate() {
        let category = categories.get(idx).copied().flatten();
        if filters.allows(category) {
            filtered_keys.push(key);
            filtered_kinds.push(kind);
        }
    }
    (filtered_keys, filtered_kinds)
}

fn snapshot_country_field<'a>(
    snap: &'a snapshot::Snapshot,
    tag: &str,
    dotted: &str,
) -> Captured<&'a serde_json::Value> {
    crate::host::context::classify_value(snap.country_path(tag, dotted))
}

fn pick_production_line_key(
    viewport: Rect,
    keys: &[Option<String>],
    scroll_y: f32,
    x: f32,
    y: f32,
) -> Option<String> {
    if x < viewport.x
        || x > viewport.x + viewport.w
        || y < viewport.y
        || y > viewport.y + viewport.h
    {
        return None;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / PRODUCTION_ROW_H).floor() as usize;
    keys.get(idx).and_then(|key| key.clone())
}

fn combine_production_line_rows(
    mil_rows: &[DataRow],
    naval_rows: &[DataRow],
    categories: &[Option<ProductionPrimaryFilter>],
) -> Vec<ProductionLineRow> {
    let mut rows = Vec::with_capacity(mil_rows.len() + naval_rows.len());
    for (idx, row) in mil_rows.iter().cloned().enumerate() {
        rows.push(ProductionLineRow {
            kind: ProductionLineKind::Military,
            primary_filter: categories.get(idx).copied().flatten(),
            row,
        });
    }
    for (idx, row) in naval_rows.iter().cloned().enumerate() {
        rows.push(ProductionLineRow {
            kind: ProductionLineKind::Naval,
            primary_filter: categories.get(mil_rows.len() + idx).copied().flatten(),
            row,
        });
    }
    rows
}

fn filter_production_line_rows(
    rows: &[ProductionLineRow],
    filters: ProductionPrimaryFilters,
) -> Vec<ProductionLineRow> {
    rows.iter()
        .filter(|row| filters.allows(row.primary_filter))
        .cloned()
        .collect()
}

/// The production view's owned per-frame draw inputs.
struct ProductionDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    mil_rows: Vec<DataRow>,
    naval_rows: Vec<DataRow>,
    line_rows: Vec<ProductionLineRow>,
    resource_rows: Vec<DataRow>,
    selected_line_key: Option<String>,
    line_window: ProductionLineWindow,
    primary_filters: ProductionPrimaryFilters,
    /// SEPARATE section states (Codex point 3): a present military list + an absent naval
    /// list must NOT be conflated — each is tagged distinctly in the digest.
    mil_section: SectionState,
    naval_section: SectionState,
}

impl Drawable for ProductionDrawable {
    fn draw_digest(&self) -> String {
        let selected_state = match self.selected_line_key.as_deref() {
            None => "none",
            Some(key)
                if self
                    .line_rows
                    .iter()
                    .any(|r| r.row.key.as_deref() == Some(key)) =>
            {
                "selected"
            }
            Some(_) => "missing_selected",
        };
        format!(
            "PROD mil_sect={} naval_sect={} filters={} selected={} selected_key={} selected_row={} scroll={:.0}/{:.0} visible=[{}] mil={} naval={} res={}",
            self.mil_section.tag(),
            self.naval_section.tag(),
            self.primary_filters.digest(),
            selected_state,
            self.selected_line_key.as_deref().unwrap_or("none"),
            selected_production_line_digest(&self.line_rows, self.selected_line_key.as_deref()),
            self.line_window.scroll_y,
            self.line_window.max_scroll,
            visible_production_line_digest(&self.line_rows, self.line_window),
            row_digest(&self.mil_rows),
            row_digest(&self.naval_rows),
            row_digest(&self.resource_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        // the resource strip + the two line grids are the data-bearing row factories.
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "resources_grid",
            "resource_strip_item",
            &self.resource_rows,
        );
        render_production_line_rows(
            fd,
            &self.roots,
            &self.file,
            "item_grid",
            &self.line_rows,
            self.line_window.scroll_y,
            self.selected_line_key.as_deref(),
        );
    }
}

impl ViewInstance for ProductionView {
    fn view_id(&self) -> ViewId {
        PRODUCTION_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = ctx
            .stage
            .gui_file(PRODUCTION_FILE)
            .cloned()
            .unwrap_or_else(empty_gui);

        // ----- top summary: operational-factory splits (n / m) -----
        let mut summary: Vec<Cell> = Vec::new();
        // Used = production.{military,naval}_factories (live, exact SUM); total =
        // economy.factories.{mil,dockyard} (save). The n/m pairing is derived.
        push_factory_split(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "operational_factories_value",
            "production.military_factories",
            "economy.factories.mil",
        );
        push_factory_split(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "operational_naval_factories_value",
            "production.naval_factories",
            "economy.factories.dockyard",
        );
        // repair-assignment split + the daily-output / efficiency aggregates are
        // runtime-computed / not stored → unavailable (no fabricated 0).
        for el in [
            "operational_naval_factories_repair_value",
            "docyards_output_value",
            "production_output_value",
            "efficiency_cap_value",
            "efficiency_growth_value",
            "production_efficiency_loss_value",
            "factory_bombing_defence_value",
        ] {
            push_unavailable(&mut fidelity, &mut summary, el);
        }

        // ----- production-line rows (live channel primary, save fallback) -----
        // each list keeps its OWN section state (a present military list must not mask an
        // absent naval list — Codex point 3).
        let (mil_rows, mil_section) = build_line_rows(
            ctx,
            tag,
            &mut fidelity,
            "military_lines",
            ProductionLineKind::Military,
        );
        let (naval_rows, naval_section) = build_line_rows(
            ctx,
            tag,
            &mut fidelity,
            "naval_lines",
            ProductionLineKind::Naval,
        );
        let line_categories = production_line_categories_from_arrays(
            snapshot_country_field(ctx.snapshot, tag, "production.military_lines"),
            snapshot_country_field(ctx.snapshot, tag, "production.naval_lines"),
            snapshot_country_field(ctx.snapshot, tag, "economy._production_lines"),
        );
        let line_rows = combine_production_line_rows(&mil_rows, &naval_rows, &line_categories);
        let filtered_line_rows = filter_production_line_rows(&line_rows, self.primary_filters);

        // ----- national resource strip -----
        let resource_rows = build_resource_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, PRODUCTION_ROOT, &summary);
        let line_window = production_grid_window(
            &roots,
            &file,
            filtered_line_rows.len(),
            self.line_scroll_y,
            production_grid_fallback(self.screen),
        );
        let drawable = ProductionDrawable {
            roots: roots.clone(),
            file,
            mil_rows,
            naval_rows,
            line_rows: filtered_line_rows,
            resource_rows,
            selected_line_key: self.selected_line_key.clone(),
            line_window,
            primary_filters: self.primary_filters,
            mil_section,
            naval_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: PRODUCTION_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
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
                let viewport = production_grid_fallback(self.screen);
                let scroll_y = self.line_scroll_y.min(max_production_scroll_fallback(
                    viewport,
                    self.line_keys.len(),
                ));
                if let Some(geom) =
                    production_scrollbar_geometry(viewport, self.line_keys.len(), scroll_y)
                {
                    if geom.thumb.contains(x, y) {
                        self.line_scroll_drag = Some(ProductionScrollDrag {
                            pointer_offset_y: y - geom.thumb.y,
                        });
                        return production_input_consumed(false);
                    }
                    if geom.track.contains(x, y) {
                        return production_input_consumed(false);
                    }
                }
                if production_add_line_button_hit(self.screen, x, y) {
                    return production_input_consumed(false);
                }
                if let Some(filter) = production_primary_filter_button_hit(self.screen, x, y) {
                    self.primary_filters.toggle(filter);
                    self.line_scroll_y = 0.0;
                    self.line_scroll_drag = None;
                    return production_input_consumed(true);
                }
                if production_status_filter_button_hit(self.screen, x, y) {
                    return production_input_consumed(false);
                }
                if production_line_close_button_hit(viewport, self.line_keys.len(), scroll_y, x, y)
                {
                    return production_input_consumed(false);
                }
                if production_factory_count_button_hit(
                    viewport,
                    self.line_keys.len(),
                    scroll_y,
                    x,
                    y,
                ) {
                    return production_input_consumed(false);
                }
                if production_priority_amount_button_hit(viewport, &self.line_kinds, scroll_y, x, y)
                {
                    return production_input_consumed(false);
                }
                if production_line_mutation_control_hit(viewport, &self.line_kinds, scroll_y, x, y)
                {
                    return production_input_consumed(false);
                }
                if let Some(key) =
                    pick_production_line_key(viewport, &self.line_keys, scroll_y, x, y)
                {
                    let changed = self.selected_line_key.as_deref() != Some(key.as_str());
                    self.selected_line_key = Some(key);
                    production_input_consumed(changed)
                } else {
                    InputOutcome::ignored()
                }
            }
            ViewInput::Wheel { x, y, dy } => {
                let viewport = production_grid_fallback(self.screen);
                if x < viewport.x
                    || x > viewport.x + viewport.w
                    || y < viewport.y
                    || y > viewport.y + viewport.h
                {
                    return InputOutcome::ignored();
                }
                let current = self.line_scroll_y;
                let next = scroll_production_offset(current, viewport, self.line_keys.len(), dy);
                self.line_scroll_y = next;
                production_input_consumed(next != current)
            }
            ViewInput::PointerMove { x: _, y, drag } if drag => {
                let drag = match self.line_scroll_drag {
                    Some(drag) => drag,
                    None => return InputOutcome::ignored(),
                };
                let viewport = production_grid_fallback(self.screen);
                let geom = match production_scrollbar_geometry(
                    viewport,
                    self.line_keys.len(),
                    self.line_scroll_y,
                ) {
                    Some(geom) => geom,
                    None => return InputOutcome::ignored(),
                };
                let current = self.line_scroll_y;
                let next = production_scroll_from_thumb_y(geom, y - drag.pointer_offset_y);
                self.line_scroll_y = next;
                production_input_consumed(next != current)
            }
            ViewInput::PointerUp {
                button: PointerButton::Left,
                x,
                y,
            } => {
                if self.line_scroll_drag.take().is_some() {
                    production_input_consumed(false)
                } else if production_add_line_button_hit(self.screen, x, y) {
                    production_input_consumed(false)
                } else if production_primary_filter_button_hit(self.screen, x, y).is_some() {
                    production_input_consumed(false)
                } else if production_status_filter_button_hit(self.screen, x, y) {
                    production_input_consumed(false)
                } else {
                    let viewport = production_grid_fallback(self.screen);
                    let scroll_y = self.line_scroll_y.min(max_production_scroll_fallback(
                        viewport,
                        self.line_keys.len(),
                    ));
                    if production_line_close_button_hit(
                        viewport,
                        self.line_keys.len(),
                        scroll_y,
                        x,
                        y,
                    ) || production_factory_count_button_hit(
                        viewport,
                        self.line_keys.len(),
                        scroll_y,
                        x,
                        y,
                    ) || production_priority_amount_button_hit(
                        viewport,
                        &self.line_kinds,
                        scroll_y,
                        x,
                        y,
                    ) || production_line_mutation_control_hit(
                        viewport,
                        &self.line_kinds,
                        scroll_y,
                        x,
                        y,
                    ) {
                        production_input_consumed(false)
                    } else {
                        InputOutcome::ignored()
                    }
                }
            }
            _ => InputOutcome::ignored(),
        }
    }

    fn rebind(&mut self, ctx: &RebindContext<'_>) {
        self.screen = Rect {
            x: 0.0,
            y: 0.0,
            w: ctx.screen.0 as f32,
            h: ctx.screen.1 as f32,
        };
        let live_military =
            snapshot_country_field(ctx.snapshot, ctx.country, "production.military_lines");
        let live_naval =
            snapshot_country_field(ctx.snapshot, ctx.country, "production.naval_lines");
        let save_lines =
            snapshot_country_field(ctx.snapshot, ctx.country, "economy._production_lines");
        let line_keys = production_line_keys_from_arrays(live_military, live_naval, save_lines);
        let line_kinds = production_line_kinds_from_arrays(live_military, live_naval, save_lines);
        let line_categories =
            production_line_categories_from_arrays(live_military, live_naval, save_lines);
        let (line_keys, line_kinds) = filter_production_line_metadata(
            line_keys,
            line_kinds,
            line_categories,
            self.primary_filters,
        );
        self.line_keys = line_keys;
        self.line_kinds = line_kinds;
        let viewport = production_grid_fallback(self.screen);
        self.line_scroll_y = self.line_scroll_y.min(max_production_scroll_fallback(
            viewport,
            self.line_keys.len(),
        ));
        if production_scrollbar_geometry(viewport, self.line_keys.len(), self.line_scroll_y)
            .is_none()
        {
            self.line_scroll_drag = None;
        }
    }

    crate::impl_view_any!();
}

/// Bind one production-line list (live `production.<kind>` primary; the save
/// `economy._production_lines` filtered to `kind` is the fallback) into rows.
fn build_line_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
    kind: &str,
    line_kind: ProductionLineKind,
) -> (Vec<DataRow>, SectionState) {
    // the `amount` (build-target) text box exists ONLY on the naval/railway templates, not
    // the military one — so bind it only for naval lines (else it would silently no-op).
    let has_amount = kind == "naval_lines";
    // 1) the LIVE channel: countries.<TAG>.production.<kind> (an array of line shapes).
    let live_path = format!("production.{kind}");
    let (live, live_state) = country_array(ctx, tag, &live_path);
    if let Some(arr) = live {
        let mut rows: Vec<DataRow> = arr
            .iter()
            .map(|l| bind_line_row(l, fidelity, has_amount, line_kind, false))
            .collect();
        suppress_duplicate_row_keys(&mut rows);
        live_state.record(fidelity);
        return (rows, live_state);
    }
    // 2) the SAVE fallback: economy._production_lines filtered to {kind}.
    let (save, save_state) = country_array(ctx, tag, "economy._production_lines");
    if let Some(arr) = save {
        let mut rows: Vec<DataRow> = arr
            .iter()
            .filter(|l| l.get("kind").and_then(|k| k.as_str()) == Some(kind))
            .map(|l| bind_line_row(l, fidelity, has_amount, line_kind, true))
            .collect();
        suppress_duplicate_row_keys(&mut rows);
        // the save channel WAS present → record it as the section state.
        save_state.record(fidelity);
        return (rows, save_state);
    }
    // 3) neither channel present → no rows, badge the absent live section.
    live_state.record(fidelity);
    (Vec::new(), live_state)
}

/// Bind ONE production line (live or save shape) into a row. Shared fields:
/// `active_factories`(exact), `produced`/`cost`(exact, progress derived),
/// `priority` (live: resisted→unavailable; save: measured→exact), `speed`(proxy),
/// `archetype`→name (derived token). `amount` is bound only when the template has the box
/// (`has_amount`, naval/railway). Per-line efficiency / MIO / total-need are unavailable.
fn bind_line_row(
    line: &serde_json::Value,
    report: &mut FidelityReport,
    has_amount: bool,
    kind: ProductionLineKind,
    save_channel: bool,
) -> DataRow {
    let mut row = match production_line_key(line, kind, save_channel) {
        Some(key) => DataRow::new().with_key(key),
        None => DataRow::new(),
    };
    // name / general_name: the equipment archetype TOKEN (derived — no catalogs join here).
    row.bind(
        report,
        "name",
        elem_field(line, "archetype"),
        Fidelity::Derived,
        fmt_token,
    );
    row.bind(
        report,
        "general_name",
        elem_field(line, "archetype"),
        Fidelity::Derived,
        fmt_token,
    );
    // active factories on the line (exact measured count).
    row.bind(
        report,
        "factories_used_txt",
        elem_field(line, "active_factories"),
        Fidelity::Exact,
        fmt_int,
    );
    // target build count (INFINITY = -1 in the save shape → show ∞) — naval/railway only.
    if has_amount {
        row.bind(
            report,
            "amount",
            elem_field(line, "amount"),
            Fidelity::Exact,
            fmt_amount,
        );
    }
    // per-line build speed (raw measured value; the in-game text is a computed rate → proxy).
    row.bind(
        report,
        "speed",
        elem_field(line, "speed"),
        Fidelity::Derived,
        fmt_speed,
    );
    // per-line priority: present (measured) in the save shape → exact; absent in the live
    // shape → renders "—" automatically (no fabrication).
    row.bind(
        report,
        "priority",
        elem_field(line, "priority"),
        Fidelity::Exact,
        fmt_int,
    );
    // progress to next unit = produced/cost (both exact; the fraction is derived).
    let frac = progress_fraction(line, "produced", "cost");
    let prog = match frac {
        Some(f) => DisplayAtom::text(
            format!("{}%", (f * 100.0).round() as i64),
            Fidelity::Derived,
        ),
        None => DisplayAtom::unavailable_because(crate::host::context::UnavailableReason::Absent),
    };
    report.record(&prog);
    row.cells.push(Cell {
        element: "progressbar".to_string(),
        atom: prog,
    });
    // runtime-computed / not-in-shape line fields → unavailable.
    for el in [
        "total_need",
        "efficiency_progressbar",
        "efficiency_progressbar_to_max",
    ] {
        row.badge_unavailable(report, el);
    }
    row
}

/// `amount`: the save shape uses -1 for "INFINITY". Render ∞ for a measured -1, else the int.
fn fmt_amount(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) if f < 0.0 => "∞".to_string(),
        Some(f) => format!("{}", f.trunc() as i64),
        None => DASH.to_string(),
    }
}

/// `speed`: the raw measured per-line speed (a proxy for the in-game units/period text).
fn fmt_speed(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) => format!("{f:.1}"),
        None => DASH.to_string(),
    }
}

/// The produced/cost progress fraction, or None if either leaf is not a measured number.
fn progress_fraction(elem: &serde_json::Value, num: &str, den: &str) -> Option<f32> {
    let p = elem.get(num).and_then(|v| v.as_f64())?;
    let c = elem.get(den).and_then(|v| v.as_f64())?;
    if c > 0.0 {
        Some((p / c).clamp(0.0, 1.0) as f32)
    } else {
        None
    }
}

/// Build the national resource-strip rows from `economy.resources.available_derived`.
/// ALL seven resource rows are ALWAYS emitted (Codex point 1): an absent/null resource
/// renders "—" via `display_with_fidelity`, never a fabricated 0 and never silently
/// dropped — so the fixed resource strip is provably fully accounted.
fn build_resource_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    const RESOURCES: [&str; 7] = [
        "steel",
        "oil",
        "aluminium",
        "tungsten",
        "chromium",
        "rubber",
        "coal",
    ];
    let mut rows = Vec::new();
    for res in RESOURCES {
        let cap = ctx.country_field(tag, &format!("economy.resources.available_derived.{res}"));
        let mut row = DataRow::new();
        row.bind(fidelity, "value", cap, Fidelity::Derived, fmt_int);
        rows.push(row);
    }
    rows
}

/// Bind an `operational_*_value` factory split `used / total` from a live numerator path
/// and a save denominator path. Either half absent renders "—" (no fabricated 0).
fn push_factory_split(
    ctx: &BindContext,
    tag: &str,
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
    element: &str,
    used_path: &str,
    total_path: &str,
) {
    let used = ctx.country_field(tag, used_path);
    let total = ctx.country_field(tag, total_path);
    let used_s = used.display_with_fidelity(Fidelity::Exact, fmt_int);
    let total_s = total.display_with_fidelity(Fidelity::Exact, fmt_int);
    // HONEST fidelity (Codex point 2): when BOTH halves are not-captured the split is not
    // derivable from anything — badge it unavailable ("—"), not a dishonest "—/—" derived.
    let atom = if used_s.is_unavailable() && total_s.is_unavailable() {
        DisplayAtom::unavailable_because(crate::host::context::UnavailableReason::Absent)
    } else {
        // the SPLIT is derived (a live numerator paired with a save denominator; a single
        // missing half still leaves a real number on the other side).
        DisplayAtom::text(
            format!("{}/{}", used_s.as_str(), total_s.as_str()),
            Fidelity::Derived,
        )
    };
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

/// Push an explicit `unavailable` summary cell (runtime-computed / not stored). "—" only.
fn push_unavailable(report: &mut FidelityReport, summary: &mut Vec<Cell>, element: &str) {
    let atom = DisplayAtom::unavailable_because(crate::host::context::UnavailableReason::Blocked);
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

fn open_production(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(ProductionView::new())
}

register_view!(ViewDescriptor {
    id: PRODUCTION_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(PRODUCTION_FILE, PRODUCTION_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::ProductionConstructionDecisions,
        label: "Production",
        order: 1,
    }),
    shortcut: None,
    open: open_production,
    policy: HostPolicy::window(ZBand::Panel),
});

register_template!(TemplateDescriptor {
    id: ViewId("production.military_line_entry"),
    gui_template: GuiSymbolRef::template(PRODUCTION_FILE, "production_military_line_entry"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("production.naval_line_entry"),
    gui_template: GuiSymbolRef::template(PRODUCTION_FILE, "production_naval_line_entry"),
    required_domains: &["countries"],
});
// the national resource-strip row factory (defined as a top-level template window ONLY in
// production.json; construction's `resources_grid` reuses the same symbol at runtime).
register_template!(TemplateDescriptor {
    id: ViewId("production.resource_strip_item"),
    gui_template: GuiSymbolRef::template(PRODUCTION_FILE, "resource_strip_item"),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 2 — CONSTRUCTION
// =====================================================================================

const CONSTRUCTION_VIEW_ID: ViewId = ViewId("production.construction");
const CONSTRUCTION_FILE: &str = "construction.json";
const CONSTRUCTION_ROOT: &str = "countryconstructionsview";
const CONSTRUCTION_ROW_H: f32 = 52.0;
const CONSTRUCTION_FACTORY_COUNT_BUTTON_SIZE: f32 = 24.0;
const CONSTRUCTION_QUEUE_BUTTON_SIZE: f32 = 24.0;
const CONSTRUCTION_TEMP_BUTTON_SIZE: f32 = 32.0;

pub struct ConstructionView {
    selected_queue_key: Option<String>,
    queue_keys: Vec<Option<String>>,
    queue_scroll_y: f32,
    queue_scroll_drag: Option<ConstructionScrollDrag>,
    screen: Rect,
}

impl ConstructionView {
    pub fn new() -> ConstructionView {
        ConstructionView {
            selected_queue_key: None,
            queue_keys: Vec::new(),
            queue_scroll_y: 0.0,
            queue_scroll_drag: None,
            screen: default_construction_screen(),
        }
    }
}

impl Default for ConstructionView {
    fn default() -> Self {
        ConstructionView::new()
    }
}

fn default_construction_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn construction_grid_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 605.0,
        y: screen.y + 213.0,
        w: 440.0,
        h: (screen.h - 213.0).max(0.0),
    }
}

fn construction_panel_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 606.0,
        y: screen.y + 78.0,
        w: 550.0,
        h: (screen.h - 78.0).max(0.0),
    }
}

fn construction_factory_count_button_rects(screen: Rect) -> [Rect; 2] {
    let panel = construction_panel_fallback(screen);
    let container_x = panel.x + panel.w - 220.0;
    let y = panel.y + 12.0 + 2.0;
    [
        Rect {
            x: container_x + 31.0,
            y,
            w: CONSTRUCTION_FACTORY_COUNT_BUTTON_SIZE,
            h: CONSTRUCTION_FACTORY_COUNT_BUTTON_SIZE,
        },
        Rect {
            x: container_x + 95.0,
            y,
            w: CONSTRUCTION_FACTORY_COUNT_BUTTON_SIZE,
            h: CONSTRUCTION_FACTORY_COUNT_BUTTON_SIZE,
        },
    ]
}

fn construction_factory_count_button_hit(screen: Rect, x: f32, y: f32) -> bool {
    construction_factory_count_button_rects(screen)
        .iter()
        .any(|rect| rect.contains(x, y))
}

fn construction_temp_button_rect(screen: Rect) -> Rect {
    let panel = construction_panel_fallback(screen);
    Rect {
        x: panel.x - 402.0,
        y: panel.y + 9.0,
        w: CONSTRUCTION_TEMP_BUTTON_SIZE,
        h: CONSTRUCTION_TEMP_BUTTON_SIZE,
    }
}

fn construction_temp_button_hit(screen: Rect, x: f32, y: f32) -> bool {
    construction_temp_button_rect(screen).contains(x, y)
}

fn construction_queue_button_rects(viewport: Rect, row_y: f32) -> [Rect; 5] {
    [
        Rect {
            x: viewport.x + 318.0,
            y: row_y + 14.0,
            w: CONSTRUCTION_QUEUE_BUTTON_SIZE,
            h: CONSTRUCTION_QUEUE_BUTTON_SIZE,
        },
        Rect {
            x: viewport.x + 341.0,
            y: row_y + 14.0,
            w: CONSTRUCTION_QUEUE_BUTTON_SIZE,
            h: CONSTRUCTION_QUEUE_BUTTON_SIZE,
        },
        Rect {
            x: viewport.x + 214.0,
            y: row_y + 3.0,
            w: CONSTRUCTION_QUEUE_BUTTON_SIZE,
            h: CONSTRUCTION_QUEUE_BUTTON_SIZE,
        },
        Rect {
            x: viewport.x + 214.0,
            y: row_y + 27.0,
            w: CONSTRUCTION_QUEUE_BUTTON_SIZE,
            h: CONSTRUCTION_QUEUE_BUTTON_SIZE,
        },
        Rect {
            x: viewport.x + 370.0,
            y: row_y + 14.0,
            w: CONSTRUCTION_QUEUE_BUTTON_SIZE,
            h: CONSTRUCTION_QUEUE_BUTTON_SIZE,
        },
    ]
}

fn construction_queue_button_hit(
    viewport: Rect,
    row_count: usize,
    scroll_y: f32,
    x: f32,
    y: f32,
) -> bool {
    if !viewport.contains(x, y) {
        return false;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / CONSTRUCTION_ROW_H).floor() as usize;
    if idx >= row_count {
        return false;
    }
    let row_y = viewport.y + idx as f32 * CONSTRUCTION_ROW_H - scroll_y.max(0.0);
    construction_queue_button_rects(viewport, row_y)
        .iter()
        .any(|rect| rect.contains(x, y))
}

fn construction_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
}

#[derive(Clone, Copy)]
struct ConstructionRowWindow {
    scroll_y: f32,
    max_scroll: f32,
    first: usize,
    visible: usize,
}

#[derive(Clone, Copy)]
struct ConstructionScrollDrag {
    pointer_offset_y: f32,
}

#[derive(Clone, Copy)]
struct ConstructionScrollbarGeometry {
    track: Rect,
    thumb: Rect,
    max_scroll: f32,
}

fn max_construction_scroll_fallback(viewport: Rect, row_count: usize) -> f32 {
    (row_count as f32 * CONSTRUCTION_ROW_H - viewport.h).max(0.0)
}

fn construction_scrollbar_geometry(
    viewport: Rect,
    row_count: usize,
    scroll_y: f32,
) -> Option<ConstructionScrollbarGeometry> {
    let content_h = row_count as f32 * CONSTRUCTION_ROW_H;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport.h, content_h);
    if max_scroll <= 0.0 {
        return None;
    }
    let track = Rect {
        x: viewport.x + viewport.w - GUTTER,
        y: viewport.y,
        w: GUTTER,
        h: viewport.h,
    };
    let visible_frac = (viewport.h / content_h).clamp(0.05, 1.0);
    let thumb_h = (track.h * visible_frac).max(16.0).min(track.h);
    let scroll_frac = if max_scroll > 0.0 {
        scroll_y.clamp(0.0, max_scroll) / max_scroll
    } else {
        0.0
    };
    let thumb_y = track.y + (track.h - thumb_h).max(0.0) * scroll_frac.clamp(0.0, 1.0);
    Some(ConstructionScrollbarGeometry {
        track,
        thumb: Rect {
            x: track.x,
            y: thumb_y,
            w: GUTTER,
            h: thumb_h,
        },
        max_scroll,
    })
}

fn construction_scroll_from_thumb_y(geom: ConstructionScrollbarGeometry, thumb_y: f32) -> f32 {
    let travel = (geom.track.h - geom.thumb.h).max(0.0);
    if travel <= 0.0 || geom.max_scroll <= 0.0 {
        return 0.0;
    }
    let y = thumb_y.clamp(geom.track.y, geom.track.y + travel);
    ((y - geom.track.y) / travel) * geom.max_scroll
}

fn scroll_construction_offset(current: f32, viewport: Rect, row_count: usize, dy: f32) -> f32 {
    if dy == 0.0 || !dy.is_finite() {
        return current;
    }
    let rows = dy.abs().ceil().max(1.0);
    let delta = rows * CONSTRUCTION_ROW_H;
    let next = if dy < 0.0 {
        current + delta
    } else {
        (current - delta).max(0.0)
    };
    next.min(max_construction_scroll_fallback(viewport, row_count))
}

fn construction_grid_window(
    roots: &[Node],
    file: &GuiFile,
    row_count: usize,
    requested_scroll: f32,
    fallback_viewport: Rect,
) -> ConstructionRowWindow {
    let metrics = find_rect(roots, "item_grid")
        .and_then(|viewport| {
            find_template(file, "production_building_line_entry").map(|t| (viewport, t))
        })
        .map(|(viewport, template)| {
            let spec = GridSpec::from_gridbox(&stub_grid("item_grid", viewport), &template, GUTTER);
            (viewport, spec.cell_h.max(1.0), spec.columns.max(1))
        });
    let (viewport_h, cell_h, columns) = match metrics {
        Some((viewport, cell_h, columns)) => (viewport.h, cell_h, columns),
        None => (fallback_viewport.h, CONSTRUCTION_ROW_H, 1),
    };
    let grid_rows = row_count.div_ceil(columns);
    let content_h = grid_rows as f32 * cell_h;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport_h, content_h);
    let scroll_y = requested_scroll.clamp(0.0, max_scroll);
    let first_grid_row = (scroll_y / cell_h).floor() as usize;
    let visible_grid_rows = (viewport_h / cell_h).ceil().max(1.0) as usize + 1;
    ConstructionRowWindow {
        scroll_y,
        max_scroll,
        first: first_grid_row.saturating_mul(columns),
        visible: visible_grid_rows.saturating_mul(columns),
    }
}

fn visible_construction_row_digest(rows: &[DataRow], window: ConstructionRowWindow) -> String {
    let visible: Vec<DataRow> = rows
        .iter()
        .skip(window.first)
        .take(window.visible)
        .cloned()
        .collect();
    row_digest(&visible)
}

fn construction_queue_key(line: &serde_json::Value, save_channel: bool) -> Option<String> {
    if save_channel {
        let template = match elem_field(line, "template") {
            Captured::Measured(v) => row_key_token(v)?,
            Captured::Null | Captured::Absent => return None,
        };
        let state = match elem_field(line, "location_state") {
            Captured::Measured(v) => row_key_token(v)?,
            Captured::Null | Captured::Absent => return None,
        };
        let created = match elem_field(line, "created_date") {
            Captured::Measured(v) => row_key_token(v)?,
            Captured::Null | Captured::Absent => return None,
        };
        return Some(format!(
            "construction:template:{template}:state:{state}:created:{created}"
        ));
    }
    match elem_field(line, "line_id") {
        Captured::Measured(v) => row_key_token(v).map(|id| format!("construction:line:{id}")),
        Captured::Null | Captured::Absent => None,
    }
}

fn construction_queue_keys_from_arrays(
    live: Captured<&serde_json::Value>,
    save: Captured<&serde_json::Value>,
) -> Vec<Option<String>> {
    let (live_arr, _) = array_from_capture(live);
    if let Some(arr) = live_arr {
        return arr
            .iter()
            .map(|line| construction_queue_key(line, false))
            .collect();
    }
    let (save_arr, _) = array_from_capture(save);
    match save_arr {
        Some(arr) => arr
            .iter()
            .map(|line| construction_queue_key(line, true))
            .collect(),
        None => Vec::new(),
    }
}

fn pick_construction_row_key(
    viewport: Rect,
    keys: &[Option<String>],
    scroll_y: f32,
    x: f32,
    y: f32,
) -> Option<String> {
    if !viewport.contains(x, y) {
        return None;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / CONSTRUCTION_ROW_H).floor() as usize;
    keys.get(idx).and_then(|key| key.clone())
}

struct ConstructionDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    queue_rows: Vec<DataRow>,
    resource_rows: Vec<DataRow>,
    section: SectionState,
    selected_queue_key: Option<String>,
    queue_window: ConstructionRowWindow,
    screen: Rect,
}

impl Drawable for ConstructionDrawable {
    fn draw_digest(&self) -> String {
        let selected_state = match self.selected_queue_key.as_deref() {
            Some(key)
                if self
                    .queue_rows
                    .iter()
                    .any(|row| row.key.as_deref() == Some(key)) =>
            {
                "selected"
            }
            Some(_) => "missing_selected",
            None => "none",
        };
        format!(
            "CONSTR sect={} selected={} selected_key={} selected_row={} scroll={:.0}/{:.0} visible=[{}] queue={} res={}",
            self.section.tag(),
            selected_state,
            self.selected_queue_key.as_deref().unwrap_or("none"),
            selected_row_digest(&self.queue_rows, self.selected_queue_key.as_deref()),
            self.queue_window.scroll_y,
            self.queue_window.max_scroll,
            visible_construction_row_digest(&self.queue_rows, self.queue_window),
            row_digest(&self.queue_rows),
            row_digest(&self.resource_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        // NOTE: `resource_strip_item` is defined as a top-level template ONLY in
        // production.json; construction.json's `resources_grid` reuses it at engine runtime
        // but our per-file template lookup can't see it here, so `render_rows` returns early
        // (no rows drawn). The resource DATA is still BOUND + recorded in the digest/fidelity
        // (`build_resource_rows`) — it is honestly accounted, just not rendered from this file
        // (a cross-file shared-template gap, not a fabrication).
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "resources_grid",
            "resource_strip_item",
            &self.resource_rows,
        );
        render_rows_selected_in_viewport(
            fd,
            &self.file,
            "item_grid",
            "production_building_line_entry",
            &self.queue_rows,
            construction_grid_fallback(self.screen),
            self.queue_window.scroll_y,
            self.selected_queue_key.as_deref(),
        );
    }
}

impl ViewInstance for ConstructionView {
    fn view_id(&self) -> ViewId {
        CONSTRUCTION_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => {
                let viewport = construction_grid_fallback(self.screen);
                let scroll_y = self.queue_scroll_y.min(max_construction_scroll_fallback(
                    viewport,
                    self.queue_keys.len(),
                ));
                if let Some(geom) =
                    construction_scrollbar_geometry(viewport, self.queue_keys.len(), scroll_y)
                {
                    if geom.thumb.contains(x, y) {
                        self.queue_scroll_drag = Some(ConstructionScrollDrag {
                            pointer_offset_y: y - geom.thumb.y,
                        });
                        return construction_input_consumed(false);
                    }
                    if geom.track.contains(x, y) {
                        return construction_input_consumed(false);
                    }
                }
                if construction_factory_count_button_hit(self.screen, x, y) {
                    return construction_input_consumed(false);
                }
                if construction_temp_button_hit(self.screen, x, y) {
                    return construction_input_consumed(false);
                }
                if construction_queue_button_hit(viewport, self.queue_keys.len(), scroll_y, x, y) {
                    return construction_input_consumed(false);
                }
                if let Some(key) =
                    pick_construction_row_key(viewport, &self.queue_keys, scroll_y, x, y)
                {
                    let changed = self.selected_queue_key.as_deref() != Some(key.as_str());
                    self.selected_queue_key = Some(key);
                    construction_input_consumed(changed)
                } else {
                    InputOutcome::ignored()
                }
            }
            ViewInput::Wheel { x, y, dy } => {
                let viewport = construction_grid_fallback(self.screen);
                if !viewport.contains(x, y) {
                    return InputOutcome::ignored();
                }
                let current = self.queue_scroll_y;
                let next = scroll_construction_offset(current, viewport, self.queue_keys.len(), dy);
                self.queue_scroll_y = next;
                construction_input_consumed(next != current)
            }
            ViewInput::PointerMove { x: _, y, drag } if drag => {
                let drag = match self.queue_scroll_drag {
                    Some(drag) => drag,
                    None => return InputOutcome::ignored(),
                };
                let viewport = construction_grid_fallback(self.screen);
                let geom = match construction_scrollbar_geometry(
                    viewport,
                    self.queue_keys.len(),
                    self.queue_scroll_y,
                ) {
                    Some(geom) => geom,
                    None => return InputOutcome::ignored(),
                };
                let current = self.queue_scroll_y;
                let next = construction_scroll_from_thumb_y(geom, y - drag.pointer_offset_y);
                self.queue_scroll_y = next;
                construction_input_consumed(next != current)
            }
            ViewInput::PointerUp {
                button: PointerButton::Left,
                x,
                y,
            } => {
                if self.queue_scroll_drag.take().is_some() {
                    construction_input_consumed(false)
                } else if construction_factory_count_button_hit(self.screen, x, y) {
                    construction_input_consumed(false)
                } else if construction_temp_button_hit(self.screen, x, y) {
                    construction_input_consumed(false)
                } else {
                    let viewport = construction_grid_fallback(self.screen);
                    let scroll_y = self.queue_scroll_y.min(max_construction_scroll_fallback(
                        viewport,
                        self.queue_keys.len(),
                    ));
                    if construction_queue_button_hit(
                        viewport,
                        self.queue_keys.len(),
                        scroll_y,
                        x,
                        y,
                    ) {
                        construction_input_consumed(false)
                    } else {
                        InputOutcome::ignored()
                    }
                }
            }
            _ => InputOutcome::ignored(),
        }
    }
    fn rebind(&mut self, ctx: &RebindContext) {
        self.screen = Rect {
            x: 0.0,
            y: 0.0,
            w: ctx.screen.0 as f32,
            h: ctx.screen.1 as f32,
        };
        self.queue_keys = construction_queue_keys_from_arrays(
            ctx.country_field(ctx.country, "production.construction_queue"),
            ctx.country_field(ctx.country, "economy.construction_queue"),
        );
        let viewport = construction_grid_fallback(self.screen);
        self.queue_scroll_y = self.queue_scroll_y.min(max_construction_scroll_fallback(
            viewport,
            self.queue_keys.len(),
        ));
        if construction_scrollbar_geometry(viewport, self.queue_keys.len(), self.queue_scroll_y)
            .is_none()
        {
            self.queue_scroll_drag = None;
        }
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = ctx
            .stage
            .gui_file(CONSTRUCTION_FILE)
            .cloned()
            .unwrap_or_else(empty_gui);

        // ----- top summary -----
        let mut summary: Vec<Cell> = Vec::new();
        // available civ factories: total = economy.factories.civ (save, exact); the free
        // split is computed → render the total as the value, derived.
        push_single(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "available_factories_value",
            "economy.factories.civ",
            Fidelity::Derived,
            fmt_int,
        );
        push_single(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "local_factories_value",
            "economy.factories.civ_from_states",
            Fidelity::Derived,
            fmt_int,
        );
        // traded = total − local (derivable, but the leaf itself is absent) → unavailable.
        push_unavailable(&mut fidelity, &mut summary, "traded_factories_value");
        // CIC bank + construction-speed modifier are computed/not stored → unavailable.
        push_unavailable(&mut fidelity, &mut summary, "cic_bank_value");
        push_unavailable(&mut fidelity, &mut summary, "construction_speed_value");

        // ----- construction-queue rows (live primary, save fallback) -----
        let (queue_rows, section) = build_queue_rows(ctx, tag, &mut fidelity);

        // ----- resource strip -----
        let resource_rows = build_resource_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, CONSTRUCTION_ROOT, &summary);
        let queue_window = construction_grid_window(
            &roots,
            &file,
            queue_rows.len(),
            self.queue_scroll_y,
            construction_grid_fallback(self.screen),
        );
        let drawable = ConstructionDrawable {
            roots: roots.clone(),
            file,
            queue_rows,
            resource_rows,
            section,
            selected_queue_key: self.selected_queue_key.clone(),
            queue_window,
            screen: self.screen,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: CONSTRUCTION_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Build the construction-queue rows. Live `production.construction_queue` is the Phase-3
/// primary (location RESISTED → unavailable); `economy.construction_queue` (save) is the
/// richer fallback (carries `template` + `location_state`).
fn build_queue_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    // 1) live channel.
    let (live, live_state) = country_array(ctx, tag, "production.construction_queue");
    if let Some(arr) = live {
        let rows = arr
            .iter()
            .map(|l| bind_queue_row(l, fidelity, false))
            .collect();
        live_state.record(fidelity);
        return (rows, live_state);
    }
    // 2) save fallback (has location_state + template).
    let (save, save_state) = country_array(ctx, tag, "economy.construction_queue");
    if let Some(arr) = save {
        let rows = arr
            .iter()
            .map(|l| bind_queue_row(l, fidelity, true))
            .collect();
        save_state.record(fidelity);
        return (rows, save_state);
    }
    live_state.record(fidelity);
    (Vec::new(), live_state)
}

/// Bind ONE construction-queue line. `save_channel` = true when the row came from
/// `economy.construction_queue` (which carries `template` + `location_state`).
fn bind_queue_row(
    line: &serde_json::Value,
    report: &mut FidelityReport,
    save_channel: bool,
) -> DataRow {
    let mut row = match construction_queue_key(line, save_channel) {
        Some(key) => DataRow::new().with_key(key),
        None => DataRow::new(),
    };
    // building picture/name: live `template_id` or save `template` token (derived icon lookup).
    let tmpl_key = if save_channel {
        "template"
    } else {
        "template_id"
    };
    row.bind(
        report,
        "picture",
        elem_field(line, tmpl_key),
        Fidelity::Derived,
        fmt_token,
    );
    // factories assigned (exact).
    row.bind(
        report,
        "factories",
        elem_field(line, "active_factories"),
        Fidelity::Exact,
        fmt_int,
    );
    // build location: live RESISTED → unavailable; save → location_state id (derived).
    if save_channel {
        row.bind(
            report,
            "location",
            elem_field(line, "location_state"),
            Fidelity::Derived,
            fmt_state_id,
        );
    } else {
        row.badge_unavailable(report, "location");
    }
    // progress = produced/cost (exact leaves; fraction derived).
    let frac = progress_fraction(line, "produced", "cost");
    let prog = match frac {
        Some(f) => DisplayAtom::text(
            format!("{}%", (f * 100.0).round() as i64),
            Fidelity::Derived,
        ),
        None => DisplayAtom::unavailable_because(crate::host::context::UnavailableReason::Absent),
    };
    report.record(&prog);
    row.cells.push(Cell {
        element: "progress".to_string(),
        atom: prog,
    });
    // queued-count + relative_flag owner: not stored / not wired → unavailable.
    row.badge_unavailable(report, "queued");
    row.badge_unavailable(report, "relative_flag");
    row
}

/// Render a measured `location_state` id as `#<id>` (derived — an id fallback, NOT a
/// localized state name). "—" if not a measured number.
fn fmt_state_id(v: &serde_json::Value) -> String {
    match v.as_i64() {
        Some(i) => format!("#{i}"),
        None => DASH.to_string(),
    }
}

/// Push a single summary cell bound from one country path at a chosen fidelity.
fn push_single(
    ctx: &BindContext,
    tag: &str,
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
    element: &str,
    path: &str,
    fidelity: Fidelity,
    fmt: impl FnOnce(&serde_json::Value) -> String,
) {
    let cap = ctx.country_field(tag, path);
    let atom = cap.display_with_fidelity(fidelity, fmt);
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

fn open_construction(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(ConstructionView::new())
}

register_view!(ViewDescriptor {
    id: CONSTRUCTION_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(CONSTRUCTION_FILE, CONSTRUCTION_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::ProductionConstructionDecisions,
        label: "Construction",
        order: 2,
    }),
    shortcut: None,
    open: open_construction,
    policy: HostPolicy::window(ZBand::Panel),
});

register_template!(TemplateDescriptor {
    id: ViewId("production.building_line_entry"),
    gui_template: GuiSymbolRef::template(CONSTRUCTION_FILE, "production_building_line_entry"),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 3 — DECISIONS
// =====================================================================================

const DECISIONS_VIEW_ID: ViewId = ViewId("production.decisions");
const DECISIONS_FILE: &str = "decisions.json";
const DECISIONS_ROOT: &str = "countrydecisionview";
const DECISION_ROW_H: f32 = 40.0;
const DECISION_SELECT_BUTTON_SIZE: f32 = 24.0;

pub struct DecisionsView {
    selected_timed_key: Option<String>,
    timed_keys: Vec<Option<String>>,
    timed_scroll_y: f32,
    timed_scroll_drag: Option<DecisionScrollDrag>,
    screen: Rect,
}

impl DecisionsView {
    pub fn new() -> DecisionsView {
        DecisionsView {
            selected_timed_key: None,
            timed_keys: Vec::new(),
            timed_scroll_y: 0.0,
            timed_scroll_drag: None,
            screen: default_decisions_screen(),
        }
    }
}

impl Default for DecisionsView {
    fn default() -> Self {
        DecisionsView::new()
    }
}

fn default_decisions_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn decisions_grid_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 592.0,
        y: screen.y + 123.0,
        w: 502.0,
        h: (screen.h - 133.0).max(0.0),
    }
}

fn decisions_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
}

fn timed_decision_key(dec: &serde_json::Value) -> Option<String> {
    let decision = match elem_field(dec, "decision") {
        Captured::Measured(v) => row_key_token(v)?,
        Captured::Null | Captured::Absent => return None,
    };
    let mut key = format!("decision:{decision}");
    if let Captured::Measured(v) = elem_field(dec, "state") {
        if let Some(state) = row_key_token(v) {
            key.push_str(":state:");
            key.push_str(&state);
        }
    }
    Some(key)
}

#[derive(Clone, Copy)]
struct DecisionRowWindow {
    scroll_y: f32,
    max_scroll: f32,
    first: usize,
    visible: usize,
}

#[derive(Clone, Copy)]
struct DecisionScrollDrag {
    pointer_offset_y: f32,
}

#[derive(Clone, Copy)]
struct DecisionScrollbarGeometry {
    track: Rect,
    thumb: Rect,
    max_scroll: f32,
}

fn max_decision_scroll_fallback(viewport: Rect, row_count: usize) -> f32 {
    (row_count as f32 * DECISION_ROW_H - viewport.h).max(0.0)
}

fn decision_scrollbar_geometry(
    viewport: Rect,
    row_count: usize,
    scroll_y: f32,
) -> Option<DecisionScrollbarGeometry> {
    let content_h = row_count as f32 * DECISION_ROW_H;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport.h, content_h);
    if max_scroll <= 0.0 {
        return None;
    }
    let track = Rect {
        x: viewport.x + viewport.w - GUTTER,
        y: viewport.y,
        w: GUTTER,
        h: viewport.h,
    };
    let visible_frac = (viewport.h / content_h).clamp(0.05, 1.0);
    let thumb_h = (track.h * visible_frac).max(16.0).min(track.h);
    let scroll_frac = if max_scroll > 0.0 {
        scroll_y.clamp(0.0, max_scroll) / max_scroll
    } else {
        0.0
    };
    let thumb_y = track.y + (track.h - thumb_h).max(0.0) * scroll_frac.clamp(0.0, 1.0);
    Some(DecisionScrollbarGeometry {
        track,
        thumb: Rect {
            x: track.x,
            y: thumb_y,
            w: GUTTER,
            h: thumb_h,
        },
        max_scroll,
    })
}

fn decision_scroll_from_thumb_y(geom: DecisionScrollbarGeometry, thumb_y: f32) -> f32 {
    let travel = (geom.track.h - geom.thumb.h).max(0.0);
    if travel <= 0.0 || geom.max_scroll <= 0.0 {
        return 0.0;
    }
    let y = thumb_y.clamp(geom.track.y, geom.track.y + travel);
    ((y - geom.track.y) / travel) * geom.max_scroll
}

fn scroll_decision_offset(current: f32, viewport: Rect, row_count: usize, dy: f32) -> f32 {
    if dy == 0.0 || !dy.is_finite() {
        return current;
    }
    let rows = dy.abs().ceil().max(1.0);
    let delta = rows * DECISION_ROW_H;
    let next = if dy < 0.0 {
        current + delta
    } else {
        (current - delta).max(0.0)
    };
    next.min(max_decision_scroll_fallback(viewport, row_count))
}

fn decision_grid_window(
    roots: &[Node],
    file: &GuiFile,
    row_count: usize,
    requested_scroll: f32,
    fallback_viewport: Rect,
) -> DecisionRowWindow {
    let metrics = find_rect(roots, "decision_grid")
        .and_then(|viewport| find_template(file, "timed_decision_item").map(|_| viewport))
        .map(|viewport| {
            let spec = decision_grid_spec(viewport);
            (viewport, spec.cell_h.max(1.0), spec.columns.max(1))
        });
    let (viewport_h, cell_h, columns) = match metrics {
        Some((viewport, cell_h, columns)) => (viewport.h, cell_h, columns),
        None => (fallback_viewport.h, DECISION_ROW_H, 1),
    };
    let grid_rows = row_count.div_ceil(columns);
    let content_h = grid_rows as f32 * cell_h;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport_h, content_h);
    let scroll_y = requested_scroll.clamp(0.0, max_scroll);
    let first_grid_row = (scroll_y / cell_h).floor() as usize;
    let visible_grid_rows = (viewport_h / cell_h).ceil().max(1.0) as usize + 1;
    DecisionRowWindow {
        scroll_y,
        max_scroll,
        first: first_grid_row.saturating_mul(columns),
        visible: visible_grid_rows.saturating_mul(columns),
    }
}

fn visible_row_digest(rows: &[DataRow], window: DecisionRowWindow) -> String {
    let visible: Vec<DataRow> = rows
        .iter()
        .skip(window.first)
        .take(window.visible)
        .cloned()
        .collect();
    row_digest(&visible)
}

fn timed_decision_keys_from_arrays(
    live: Captured<&serde_json::Value>,
    save: Captured<&serde_json::Value>,
) -> Vec<Option<String>> {
    let (live_arr, _) = array_from_capture(live);
    if let Some(arr) = live_arr {
        return arr.iter().map(timed_decision_key).collect();
    }
    let (save_arr, _) = array_from_capture(save);
    match save_arr {
        Some(arr) => arr.iter().map(timed_decision_key).collect(),
        None => Vec::new(),
    }
}

fn pick_decision_row_key(
    viewport: Rect,
    keys: &[Option<String>],
    scroll_y: f32,
    x: f32,
    y: f32,
) -> Option<String> {
    if !viewport.contains(x, y) {
        return None;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / DECISION_ROW_H).floor() as usize;
    keys.get(idx).and_then(|key| key.clone())
}

fn decision_select_button_rect(viewport: Rect, row_y: f32) -> Rect {
    Rect {
        x: viewport.x + 465.0,
        y: row_y + 6.0,
        w: DECISION_SELECT_BUTTON_SIZE,
        h: DECISION_SELECT_BUTTON_SIZE,
    }
}

fn decision_select_button_hit(
    viewport: Rect,
    row_count: usize,
    scroll_y: f32,
    x: f32,
    y: f32,
) -> bool {
    if !viewport.contains(x, y) {
        return false;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / DECISION_ROW_H).floor() as usize;
    if idx >= row_count {
        return false;
    }
    let row_y = viewport.y + idx as f32 * DECISION_ROW_H - scroll_y.max(0.0);
    decision_select_button_rect(viewport, row_y).contains(x, y)
}

struct DecisionsDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    timed_rows: Vec<DataRow>,
    timed_window: DecisionRowWindow,
    section: SectionState,
    selected_timed_key: Option<String>,
}

impl Drawable for DecisionsDrawable {
    fn draw_digest(&self) -> String {
        let selected_key = match self.selected_timed_key.as_deref() {
            Some(key) => key,
            None => DASH,
        };
        let selected_state = match self.selected_timed_key.as_deref() {
            Some(key)
                if self
                    .timed_rows
                    .iter()
                    .any(|row| row.key.as_deref() == Some(key)) =>
            {
                "selected"
            }
            Some(_) => "missing_selected",
            None => "none",
        };
        format!(
            "DECIS sect={} selected={} selected_key={} selected_row={} scroll={:.0}/{:.0} visible=[{}] timed={}",
            self.section.tag(),
            selected_state,
            selected_key,
            selected_row_digest(&self.timed_rows, self.selected_timed_key.as_deref()),
            self.timed_window.scroll_y,
            self.timed_window.max_scroll,
            visible_row_digest(&self.timed_rows, self.timed_window),
            row_digest(&self.timed_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        let viewport = match find_rect(&self.roots, "decision_grid") {
            Some(rect) => rect,
            None => return,
        };
        render_rows_selected_in_viewport(
            fd,
            &self.file,
            "decision_grid",
            "timed_decision_item",
            &self.timed_rows,
            viewport,
            self.timed_window.scroll_y,
            self.selected_timed_key.as_deref(),
        );
    }
}

impl ViewInstance for DecisionsView {
    fn view_id(&self) -> ViewId {
        DECISIONS_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn rebind(&mut self, ctx: &RebindContext) {
        self.screen = Rect {
            x: 0.0,
            y: 0.0,
            w: ctx.screen.0 as f32,
            h: ctx.screen.1 as f32,
        };
        self.timed_keys = timed_decision_keys_from_arrays(
            ctx.country_field(ctx.country, "decisions.timed_live"),
            ctx.country_field(ctx.country, "decisions.timed"),
        );
        let viewport = decisions_grid_fallback(self.screen);
        self.timed_scroll_y = self.timed_scroll_y.min(max_decision_scroll_fallback(
            viewport,
            self.timed_keys.len(),
        ));
        if decision_scrollbar_geometry(viewport, self.timed_keys.len(), self.timed_scroll_y)
            .is_none()
        {
            self.timed_scroll_drag = None;
        }
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => {
                let viewport = decisions_grid_fallback(self.screen);
                let scroll_y = self.timed_scroll_y.min(max_decision_scroll_fallback(
                    viewport,
                    self.timed_keys.len(),
                ));
                if let Some(geom) =
                    decision_scrollbar_geometry(viewport, self.timed_keys.len(), scroll_y)
                {
                    if geom.thumb.contains(x, y) {
                        self.timed_scroll_drag = Some(DecisionScrollDrag {
                            pointer_offset_y: y - geom.thumb.y,
                        });
                        return decisions_input_consumed(false);
                    }
                    if geom.track.contains(x, y) {
                        return decisions_input_consumed(false);
                    }
                }
                if decision_select_button_hit(viewport, self.timed_keys.len(), scroll_y, x, y) {
                    return decisions_input_consumed(false);
                }
                if let Some(key) = pick_decision_row_key(viewport, &self.timed_keys, scroll_y, x, y)
                {
                    let changed = self.selected_timed_key.as_deref() != Some(key.as_str());
                    self.selected_timed_key = Some(key);
                    decisions_input_consumed(changed)
                } else {
                    InputOutcome::ignored()
                }
            }
            ViewInput::Wheel { x, y, dy } => {
                let viewport = decisions_grid_fallback(self.screen);
                if !viewport.contains(x, y) {
                    return InputOutcome::ignored();
                }
                let current = self.timed_scroll_y;
                let next = scroll_decision_offset(current, viewport, self.timed_keys.len(), dy);
                self.timed_scroll_y = next;
                decisions_input_consumed(next != current)
            }
            ViewInput::PointerMove { x: _, y, drag } if drag => {
                let drag = match self.timed_scroll_drag {
                    Some(drag) => drag,
                    None => return InputOutcome::ignored(),
                };
                let viewport = decisions_grid_fallback(self.screen);
                let geom = match decision_scrollbar_geometry(
                    viewport,
                    self.timed_keys.len(),
                    self.timed_scroll_y,
                ) {
                    Some(geom) => geom,
                    None => return InputOutcome::ignored(),
                };
                let current = self.timed_scroll_y;
                let next = decision_scroll_from_thumb_y(geom, y - drag.pointer_offset_y);
                self.timed_scroll_y = next;
                decisions_input_consumed(next != current)
            }
            ViewInput::PointerUp {
                button: PointerButton::Left,
                x,
                y,
            } => {
                if self.timed_scroll_drag.take().is_some() {
                    decisions_input_consumed(false)
                } else {
                    let viewport = decisions_grid_fallback(self.screen);
                    let scroll_y = self.timed_scroll_y.min(max_decision_scroll_fallback(
                        viewport,
                        self.timed_keys.len(),
                    ));
                    if decision_select_button_hit(viewport, self.timed_keys.len(), scroll_y, x, y) {
                        decisions_input_consumed(false)
                    } else {
                        InputOutcome::ignored()
                    }
                }
            }
            _ => InputOutcome::ignored(),
        }
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = ctx
            .stage
            .gui_file(DECISIONS_FILE)
            .cloned()
            .unwrap_or_else(empty_gui);

        let (timed_rows, section) = build_timed_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, DECISIONS_ROOT, &[]);
        let timed_window = decision_grid_window(
            &roots,
            &file,
            timed_rows.len(),
            self.timed_scroll_y,
            decisions_grid_fallback(self.screen),
        );
        let drawable = DecisionsDrawable {
            roots: roots.clone(),
            file,
            timed_rows,
            timed_window,
            section,
            selected_timed_key: self.selected_timed_key.clone(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: DECISIONS_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Build the active TIMED-decision rows from the live `decisions.timed_live` channel; the
/// save `decisions.timed` is the fallback (one-shot AVAILABILITY is never stored, so the
/// broader catalog stays unavailable — only active timed decisions surface).
fn build_timed_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    // the per-COUNTRY decision-status presence flag (live country_containers fold), now
    // recorded at `countries.<TAG>.decisions.status_present`. Resolved ONCE here (it is
    // country-scoped, not per-decision) and surfaced on each row's `decision_status` gap:
    // Measured (the country has a decision-status record) shows the present-status object,
    // Absent/Null still badges "—" (no fabrication).
    let status_present = ctx.country_field(tag, "decisions.status_present");
    // 1) live timed-decision market.
    let (live, live_state) = country_array(ctx, tag, "decisions.timed_live");
    if let Some(arr) = live {
        let rows = arr
            .iter()
            .map(|d| bind_timed_row(d, status_present, fidelity))
            .collect();
        live_state.record(fidelity);
        return (rows, live_state);
    }
    // 2) save fallback (decisions.timed).
    let (save, save_state) = country_array(ctx, tag, "decisions.timed");
    if let Some(arr) = save {
        let rows = arr
            .iter()
            .map(|d| bind_timed_row(d, status_present, fidelity))
            .collect();
        save_state.record(fidelity);
        return (rows, save_state);
    }
    live_state.record(fidelity);
    (Vec::new(), live_state)
}

/// Bind ONE active timed decision. `decision` id → name (derived loc lookup token);
/// `days` → timer (exact); the recurring COST + the dynamic-modifier good/bad progress +
/// the `decision_status` extraction gap are not stored → unavailable.
fn bind_timed_row(
    dec: &serde_json::Value,
    status_present: Captured<&serde_json::Value>,
    report: &mut FidelityReport,
) -> DataRow {
    let mut row = match timed_decision_key(dec) {
        Some(key) => DataRow::new().with_key(key),
        None => DataRow::new(),
    };
    // name_text: the decision id token (derived — the localized name needs a decisions DB).
    row.bind(
        report,
        "name_text",
        elem_field(dec, "decision"),
        Fidelity::Derived,
        fmt_token,
    );
    // timer_text / cost_and_timer_text: days remaining (exact measured).
    row.bind(
        report,
        "timer_text",
        elem_field(dec, "days"),
        Fidelity::Exact,
        fmt_int,
    );
    row.bind(
        report,
        "cost_and_timer_text",
        elem_field(dec, "days"),
        Fidelity::Exact,
        fmt_int,
    );
    // decision_status: now FED by the live `decisions.status_present` country flag — a
    // logical gap with NO backing `.gui` element, so it is recorded (not pushed as a cell)
    // but reflects the MEASURED present-status set when the country has one, else "—".
    // Derived: the live flag is the presence of a status record, surfaced as a status token.
    row.bind_gap(
        report,
        "decision_status",
        status_present,
        Fidelity::Derived,
        fmt_status_present,
    );
    // the good/bad dynamic-modifier tick overlay (real `.gui` icons) is not stored → unavailable.
    row.badge_unavailable(report, "btn_progress_good");
    row.badge_unavailable(report, "btn_progress_bad");
    row
}

fn open_decisions(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(DecisionsView::new())
}

register_view!(ViewDescriptor {
    id: DECISIONS_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(DECISIONS_FILE, DECISIONS_ROOT)),
    required_domains: &["countries", "decisions"],
    route: Some(MenuRoute {
        cluster: MenuCluster::ProductionConstructionDecisions,
        label: "Decisions",
        order: 3,
    }),
    shortcut: None,
    open: open_decisions,
    policy: HostPolicy::window(ZBand::Panel),
});

register_template!(TemplateDescriptor {
    id: ViewId("production.timed_decision_item"),
    gui_template: GuiSymbolRef::template(DECISIONS_FILE, "timed_decision_item"),
    required_domains: &["countries", "decisions"],
});

/// Linkage anchor (kept symmetric with the other view modules; the binary calls a per-
/// cluster `link()` to force the `inventory::submit!`s into the final binary).
pub fn link() {}

// =====================================================================================
// TESTS — render-plan digest + per-view element-accounting (walk == element_type_counts).
// =====================================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::Stage;
    use crate::views::digest::bound_digest;
    use gui_runtime::model::{Coord, RawNode};
    use std::collections::HashMap;
    use std::path::Path;

    const CONSTRUCTION_FACTORY_COUNT_BUTTON_NAMES: &[&str] = &[
        "factories_sub_1x",
        "factories_sub_5x",
        "factories_sub_10x",
        "factories_add_1x",
        "factories_add_5x",
        "factories_add_10x",
    ];
    const CONSTRUCTION_QUEUE_BUTTON_NAMES: &[&str] = &[
        "increase_priority",
        "decrease_priority",
        "increase_amount",
        "decrease_amount",
        "close_production_line",
    ];
    const CONSTRUCTION_TEMP_BUTTON_NAME: &str = "temp";
    const PRODUCTION_FACTORY_COUNT_BUTTON_NAMES: &[&str] = CONSTRUCTION_FACTORY_COUNT_BUTTON_NAMES;
    const PRODUCTION_PRIORITY_AMOUNT_BUTTON_NAMES: &[&str] = &[
        "increase_priority",
        "decrease_priority",
        "btn_x10",
        "btn_x5",
        "increase_amount",
        "decrease_amount",
    ];
    const PRODUCTION_LINE_MUTATION_BUTTON_NAMES: &[(ProductionLineKind, &str)] = &[
        (ProductionLineKind::Military, "attach_manufacturer"),
        (ProductionLineKind::Military, "change_manufacturer"),
        (ProductionLineKind::Military, "equipment_button"),
        (ProductionLineKind::Military, "expand"),
        (ProductionLineKind::Military, "convert"),
        (ProductionLineKind::Naval, "attach_manufacturer"),
        (ProductionLineKind::Naval, "change_manufacturer"),
        (ProductionLineKind::Naval, "equipment_button"),
        (ProductionLineKind::Naval, "expand"),
        (ProductionLineKind::Naval, "carrier_air_wing_composition"),
        (ProductionLineKind::Naval, "deploy_location"),
        (ProductionLineKind::Naval, "name_list_btn"),
    ];
    const PRODUCTION_ADD_LINE_BUTTON_NAMES: &[&str] = &[
        "infantry_button",
        "tank_button",
        "aircraft_button",
        "naval_button",
        "naval_repair_button",
    ];
    const PRODUCTION_PRIMARY_FILTER_BUTTON_NAMES: &[(&str, ProductionPrimaryFilter)] = &[
        ("show_equipment", ProductionPrimaryFilter::Equipment),
        ("show_armor", ProductionPrimaryFilter::Armor),
        ("show_air", ProductionPrimaryFilter::Air),
        ("show_capital_ships", ProductionPrimaryFilter::CapitalShips),
        ("show_screen_ships", ProductionPrimaryFilter::ScreenShips),
        ("show_other_ships", ProductionPrimaryFilter::OtherShips),
    ];
    const PRODUCTION_STATUS_FILTER_BUTTON_NAMES: &[(&str, ProductionStatusFilter)] = &[
        ("show_resources", ProductionStatusFilter::Resources),
        ("show_outdated", ProductionStatusFilter::Outdated),
        ("show_stockpiling", ProductionStatusFilter::Stockpiling),
        ("show_reinforcing", ProductionStatusFilter::Reinforcing),
        ("show_upgrading", ProductionStatusFilter::Upgrading),
        ("show_refitting", ProductionStatusFilter::Refitting),
    ];

    /// The `game_assets/gui/` dir (READ-ONLY repo data), reached from the crate root.
    fn gui_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("game_assets")
            .join("gui")
    }

    fn load_gui(name: &str) -> Option<GuiFile> {
        let p = gui_dir().join(name);
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| GuiFile::parse(&s).ok())
    }

    fn find_raw_node<'a>(nodes: &'a [RawNode], name: &str) -> Option<&'a RawNode> {
        fn walk<'a>(node: &'a RawNode, name: &str) -> Option<&'a RawNode> {
            if node.name.as_deref() == Some(name) {
                return Some(node);
            }
            node.children.iter().find_map(|child| walk(child, name))
        }
        nodes.iter().find_map(|node| walk(node, name))
    }

    fn find_raw_window<'a>(nodes: &'a [RawNode], name: &str) -> Option<&'a RawNode> {
        nodes.iter().find(|node| node.name.as_deref() == Some(name))
    }

    fn find_raw_child<'a>(node: &'a RawNode, name: &str) -> Option<&'a RawNode> {
        fn walk<'a>(node: &'a RawNode, name: &str) -> Option<&'a RawNode> {
            if node.name.as_deref() == Some(name) {
                return Some(node);
            }
            node.children.iter().find_map(|child| walk(child, name))
        }
        node.children.iter().find_map(|child| walk(child, name))
    }

    fn stage_with(files: &[&str]) -> Stage {
        let mut gui_files = HashMap::new();
        for f in files {
            if let Some(g) = load_gui(f) {
                gui_files.insert(f.to_string(), g);
            }
        }
        Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest: assets::SpriteManifest {
                sprites: HashMap::new(),
            },
            loc: Loc::new(),
            gui_files,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
        }
    }

    #[test]
    fn production_gui_defines_resolve_factory_control_anchors() {
        let Some(gui) = load_gui(PRODUCTION_FILE) else {
            eprintln!("[production] skipped anchor test: game_assets/gui not extracted");
            return;
        };

        for (name, expected) in [
            ("prod_icon_pos_x", 358.0),
            ("prod_sub_pos_x", 379.0),
            ("prod_add_pos_x", 453.0),
            ("prod_nof_pos_x", 403.0),
        ] {
            assert_eq!(
                gui.defines.get(name).copied(),
                Some(expected),
                "{name} must be preserved from countryproductionlineview.gui"
            );
        }

        for (node_name, expected) in [
            ("factories_used", 358.0),
            ("factories_used_txt", 403.0),
            ("factories_sub_1x", 379.0),
            ("factories_add_1x", 453.0),
        ] {
            let node = find_raw_node(&gui.windows, node_name)
                .unwrap_or_else(|| panic!("{node_name} exists in production layout"));
            let coord = &node
                .position
                .as_ref()
                .unwrap_or_else(|| panic!("{node_name} carries a real position"))
                .x;
            assert!(
                matches!(coord, Coord::Anchor { resolved: Some(v), .. } if *v == expected),
                "{node_name} x coordinate must resolve to {expected}, got {coord:?}"
            );
        }
    }

    #[test]
    fn production_factory_count_button_rects_match_resolved_gui_positions() {
        let Some(gui) = load_gui(PRODUCTION_FILE) else {
            eprintln!("[production] skipped factory geometry test: game_assets/gui not extracted");
            return;
        };
        let viewport = production_grid_fallback(default_production_screen());
        let rects = production_factory_count_button_rects(viewport, viewport.y);

        for (node_name, rect) in [
            ("factories_sub_1x", rects[0]),
            ("factories_add_1x", rects[1]),
        ] {
            let node = find_raw_node(&gui.windows, node_name)
                .unwrap_or_else(|| panic!("{node_name} exists in production layout"));
            let pos = node
                .position
                .as_ref()
                .unwrap_or_else(|| panic!("{node_name} carries a real position"));
            let Coord::Anchor {
                resolved: Some(gui_x),
                ..
            } = &pos.x
            else {
                panic!("{node_name} x coordinate must be a resolved @prod_* anchor");
            };
            assert_eq!(
                rect.x,
                viewport.x + *gui_x,
                "{node_name} input rect must use the resolved GUI x coordinate"
            );
            assert_eq!(
                rect.y,
                viewport.y + pos.y.px(),
                "{node_name} input rect must use the GUI y coordinate"
            );
        }
    }

    #[test]
    fn production_add_line_button_rects_match_gui_positions() {
        let Some(gui) = load_gui(PRODUCTION_FILE) else {
            eprintln!("[production] skipped add-line geometry test: game_assets/gui not extracted");
            return;
        };
        let screen = default_production_screen();
        let panel = production_panel_fallback(screen);
        let top_y = panel.y + 30.0 + 14.0;
        let rects = production_add_line_button_rects(screen);

        for (node_name, rect) in PRODUCTION_ADD_LINE_BUTTON_NAMES.iter().zip(rects) {
            let node = find_raw_node(&gui.windows, node_name)
                .unwrap_or_else(|| panic!("{node_name} exists in production layout"));
            let pos = node
                .position
                .as_ref()
                .unwrap_or_else(|| panic!("{node_name} carries a real position"));
            assert_eq!(
                rect.x,
                panel.x + pos.x.px(),
                "{node_name} input rect must use the GUI x coordinate"
            );
            assert_eq!(
                rect.y,
                top_y + pos.y.px(),
                "{node_name} input rect must use the GUI y coordinate"
            );
        }
    }

    #[test]
    fn production_primary_filter_button_rects_match_gui_positions() {
        let Some(gui) = load_gui(PRODUCTION_FILE) else {
            eprintln!(
                "[production] skipped primary-filter geometry test: game_assets/gui not extracted"
            );
            return;
        };
        let screen = default_production_screen();
        let panel = production_panel_fallback(screen);
        let top_y = panel.y + 30.0 + 14.0;
        let rects = production_primary_filter_button_rects(screen);

        for ((node_name, filter), (rect_filter, rect)) in
            PRODUCTION_PRIMARY_FILTER_BUTTON_NAMES.iter().zip(rects)
        {
            assert_eq!(*filter, rect_filter);
            let node = find_raw_node(&gui.windows, node_name)
                .unwrap_or_else(|| panic!("{node_name} exists in production layout"));
            let pos = node
                .position
                .as_ref()
                .unwrap_or_else(|| panic!("{node_name} carries a real position"));
            assert_eq!(
                rect.x,
                panel.x + pos.x.px(),
                "{node_name} input rect must use the GUI x coordinate"
            );
            assert_eq!(
                rect.y,
                top_y + pos.y.px(),
                "{node_name} input rect must use the GUI y coordinate"
            );
            assert_eq!((rect.w, rect.h), (45.0, 32.0));
        }
    }

    #[test]
    fn production_status_filter_button_rects_match_gui_positions() {
        let Some(gui) = load_gui(PRODUCTION_FILE) else {
            eprintln!(
                "[production] skipped status-filter geometry test: game_assets/gui not extracted"
            );
            return;
        };
        let screen = default_production_screen();
        let panel = production_panel_fallback(screen);
        let top_y = panel.y + 30.0 + 14.0;
        let rects = production_status_filter_button_rects(screen);

        for ((node_name, filter), (rect_filter, rect)) in
            PRODUCTION_STATUS_FILTER_BUTTON_NAMES.iter().zip(rects)
        {
            assert_eq!(*filter, rect_filter);
            let node = find_raw_node(&gui.windows, node_name)
                .unwrap_or_else(|| panic!("{node_name} exists in production layout"));
            let pos = node
                .position
                .as_ref()
                .unwrap_or_else(|| panic!("{node_name} carries a real position"));
            assert_eq!(
                rect.x,
                panel.x + pos.x.px(),
                "{node_name} input rect must use the GUI x coordinate"
            );
            assert_eq!(
                rect.y,
                top_y + pos.y.px(),
                "{node_name} input rect must use the GUI y coordinate"
            );
            let expected_h = if *filter == ProductionStatusFilter::Refitting {
                35.0
            } else {
                32.0
            };
            assert_eq!((rect.w, rect.h), (45.0, expected_h));
        }
    }

    #[test]
    fn production_primary_filter_catalog_resolves_equipment_types_without_prefix_guessing() {
        let catalog = EquipmentTypeCatalog::get();
        for (token, kind, expected) in [
            (
                "infantry_equipment_1",
                ProductionLineKind::Military,
                ProductionPrimaryFilter::Equipment,
            ),
            (
                "medium_tank_chassis_1",
                ProductionLineKind::Military,
                ProductionPrimaryFilter::Armor,
            ),
            (
                "fighter_equipment_0",
                ProductionLineKind::Military,
                ProductionPrimaryFilter::Air,
            ),
            (
                "CAS_equipment_1",
                ProductionLineKind::Military,
                ProductionPrimaryFilter::Air,
            ),
            (
                "anti_air_equipment_2",
                ProductionLineKind::Military,
                ProductionPrimaryFilter::Equipment,
            ),
            (
                "ship_hull_heavy_1",
                ProductionLineKind::Naval,
                ProductionPrimaryFilter::CapitalShips,
            ),
            (
                "ship_hull_light_1",
                ProductionLineKind::Naval,
                ProductionPrimaryFilter::ScreenShips,
            ),
            (
                "ship_hull_submarine_1",
                ProductionLineKind::Naval,
                ProductionPrimaryFilter::OtherShips,
            ),
        ] {
            assert_eq!(
                catalog.primary_filter(token, kind),
                Some(expected),
                "{token} must classify through the equipment_archetypes catalog"
            );
        }
        assert_eq!(
            catalog.primary_filter(
                "unknown_prefix_fighter_tank_ship",
                ProductionLineKind::Military
            ),
            None,
            "unknown tokens must not be classified by string prefixes"
        );
    }

    #[test]
    fn production_priority_amount_button_rects_match_gui_positions() {
        let Some(gui) = load_gui(PRODUCTION_FILE) else {
            eprintln!(
                "[production] skipped priority/amount geometry test: game_assets/gui not extracted"
            );
            return;
        };
        let viewport = production_grid_fallback(default_production_screen());
        let row_y = viewport.y;
        let military = find_raw_window(&gui.windows, "production_military_line_entry")
            .expect("production_military_line_entry template exists");
        let naval = find_raw_window(&gui.windows, "production_naval_line_entry")
            .expect("production_naval_line_entry template exists");

        for (template, node_name, rect) in [
            (
                military,
                "increase_priority",
                production_priority_button_rects(viewport, row_y)[0],
            ),
            (
                military,
                "decrease_priority",
                production_priority_button_rects(viewport, row_y)[1],
            ),
            (
                military,
                "btn_x10",
                production_amount_button_rects(viewport, row_y, ProductionLineKind::Military)[0],
            ),
            (
                military,
                "btn_x5",
                production_amount_button_rects(viewport, row_y, ProductionLineKind::Military)[1],
            ),
            (
                naval,
                "increase_amount",
                production_amount_button_rects(viewport, row_y, ProductionLineKind::Naval)[0],
            ),
            (
                naval,
                "decrease_amount",
                production_amount_button_rects(viewport, row_y, ProductionLineKind::Naval)[1],
            ),
        ] {
            let node = find_raw_child(template, node_name).unwrap_or_else(|| {
                panic!(
                    "{node_name} exists in {}",
                    template.name.as_deref().unwrap_or("<unnamed>")
                )
            });
            let pos = node
                .position
                .as_ref()
                .unwrap_or_else(|| panic!("{node_name} carries a real position"));
            assert_eq!(
                rect.x,
                viewport.x + pos.x.px(),
                "{node_name} input rect must use the GUI x coordinate"
            );
            assert_eq!(
                rect.y,
                row_y + pos.y.px(),
                "{node_name} input rect must use the GUI y coordinate"
            );
        }
    }

    #[test]
    fn production_line_mutation_control_rects_match_gui_positions() {
        let Some(gui) = load_gui(PRODUCTION_FILE) else {
            eprintln!(
                "[production] skipped line mutation-control geometry test: game_assets/gui not extracted"
            );
            return;
        };
        let viewport = production_grid_fallback(default_production_screen());
        let row_y = viewport.y;
        let military = find_raw_window(&gui.windows, "production_military_line_entry")
            .expect("production_military_line_entry template exists");
        let naval = find_raw_window(&gui.windows, "production_naval_line_entry")
            .expect("production_naval_line_entry template exists");

        let military_rects =
            production_line_mutation_control_rects(viewport, row_y, ProductionLineKind::Military);
        let naval_rects =
            production_line_mutation_control_rects(viewport, row_y, ProductionLineKind::Naval);
        let cases = [
            (military, "attach_manufacturer", military_rects[0]),
            (military, "change_manufacturer", military_rects[1]),
            (military, "equipment_button", military_rects[2]),
            (military, "expand", military_rects[3]),
            (military, "convert", military_rects[4]),
            (naval, "attach_manufacturer", naval_rects[0]),
            (naval, "change_manufacturer", naval_rects[1]),
            (naval, "equipment_button", naval_rects[2]),
            (naval, "expand", naval_rects[3]),
            (naval, "carrier_air_wing_composition", naval_rects[4]),
            (naval, "deploy_location", naval_rects[5]),
            (naval, "name_list_btn", naval_rects[6]),
        ];

        for (template, node_name, rect) in cases {
            let node = find_raw_child(template, node_name).unwrap_or_else(|| {
                panic!(
                    "{node_name} exists in {}",
                    template.name.as_deref().unwrap_or("<unnamed>")
                )
            });
            assert_eq!(node.kind, "buttonType");
            let pos = node
                .position
                .as_ref()
                .unwrap_or_else(|| panic!("{node_name} carries a real position"));
            assert_eq!(
                rect.x,
                viewport.x + pos.x.px(),
                "{node_name} input rect must use the GUI x coordinate"
            );
            assert_eq!(
                rect.y,
                row_y + pos.y.px(),
                "{node_name} input rect must use the GUI y coordinate"
            );
        }
        assert_eq!(
            (military_rects[2].w, military_rects[2].h),
            (PRODUCTION_EQUIPMENT_BUTTON_W, PRODUCTION_EQUIPMENT_BUTTON_H),
            "equipment_button is sprite-sized in the GUI; the replay owns the large drag/drop hit area"
        );
    }

    #[test]
    fn construction_queue_button_rects_match_gui_positions() {
        let Some(gui) = load_gui(CONSTRUCTION_FILE) else {
            eprintln!(
                "[construction] skipped queue-button geometry test: game_assets/gui not extracted"
            );
            return;
        };
        let viewport = construction_grid_fallback(default_construction_screen());
        let row_y = viewport.y;
        let template = find_raw_window(&gui.windows, "production_building_line_entry")
            .expect("production_building_line_entry template exists");

        for (node_name, rect) in [
            (
                "increase_priority",
                construction_queue_button_rects(viewport, row_y)[0],
            ),
            (
                "decrease_priority",
                construction_queue_button_rects(viewport, row_y)[1],
            ),
            (
                "increase_amount",
                construction_queue_button_rects(viewport, row_y)[2],
            ),
            (
                "decrease_amount",
                construction_queue_button_rects(viewport, row_y)[3],
            ),
            (
                "close_production_line",
                construction_queue_button_rects(viewport, row_y)[4],
            ),
        ] {
            let node = find_raw_child(template, node_name)
                .unwrap_or_else(|| panic!("{node_name} exists in production_building_line_entry"));
            let pos = node
                .position
                .as_ref()
                .unwrap_or_else(|| panic!("{node_name} carries a real position"));
            assert_eq!(
                rect.x,
                viewport.x + pos.x.px(),
                "{node_name} input rect must use the GUI x coordinate"
            );
            assert_eq!(
                rect.y,
                row_y + pos.y.px(),
                "{node_name} input rect must use the GUI y coordinate"
            );
        }
    }

    #[test]
    fn construction_temp_button_rect_matches_gui_position() {
        let Some(gui) = load_gui(CONSTRUCTION_FILE) else {
            eprintln!(
                "[construction] skipped temp-button geometry test: game_assets/gui not extracted"
            );
            return;
        };
        let screen = default_construction_screen();
        let panel = construction_panel_fallback(screen);
        let rect = construction_temp_button_rect(screen);
        let node = find_raw_node(&gui.windows, CONSTRUCTION_TEMP_BUTTON_NAME)
            .expect("construction temp/backhoe button exists in root layout");
        assert_eq!(node.kind, "buttonType");
        assert_eq!(node.sprite.as_deref(), Some("GFX_backhoe_icon_small"));
        assert!(
            node.size.is_none(),
            "temp button is sprite-sized in the GUI; the fallback rect owns the hit size"
        );
        let pos = node
            .position
            .as_ref()
            .expect("temp button carries a real position");
        assert_eq!(
            rect.x,
            panel.x + pos.x.px(),
            "temp button input rect must use the GUI x coordinate"
        );
        assert_eq!(
            rect.y,
            panel.y + pos.y.px(),
            "temp button input rect must use the GUI y coordinate"
        );
        assert_eq!(
            (rect.w, rect.h),
            (CONSTRUCTION_TEMP_BUTTON_SIZE, CONSTRUCTION_TEMP_BUTTON_SIZE)
        );
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
            date: "1941.1.1.12",
            cursor: (0.0, 0.0),
            map_camera: None,
        }
    }

    /// A SAVE-backed GER snapshot mirroring the real demo: live `production`/`timed_live`
    /// ABSENT, the `economy._production_lines` + `economy.construction_queue` +
    /// `economy.factories`/`resources` + empty `decisions` lists present.
    fn snap_save() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12", "hour_index": 100,
            "countries": { "GER": {
                "economy": {
                    "factories": {"civ": 110, "mil": 131, "dockyard": 16,
                        "civ_from_states": 108, "under_construction": 12},
                    "construction_queue": [
                        {"template": "arms_factory", "location_state": 75, "active_factories": 15,
                         "produced": 6677.17, "cost": 7200.0, "priority": 0, "created_date": "1940.8.3.13"}
                    ],
                    "resources": { "available_derived": {"steel": 1196.0, "oil": 14.0, "coal": 617.0} },
                    "_production_lines": [
                        {"kind": "military_lines", "line_id": 318, "equip_id": 1855, "archetype": "medium_tank_chassis_1",
                         "produced": 4.6, "active_factories": 25, "requested_factories": 25, "priority": 0,
                         "amount": -1, "speed": 137.3, "cost": 16.0},
                        {"kind": "naval_lines", "line_id": 401, "equip_id": 900, "archetype": "ship_hull_light_1",
                         "produced": 100.0, "active_factories": 4, "requested_factories": 4, "priority": 1,
                         "amount": 3, "speed": 20.0, "cost": 800.0}
                    ]
                },
                "decisions": { "active": [], "timed": [], "missions": [] }
            } }
        }))
        .unwrap()
    }

    /// A LIVE-backed snapshot: `production.{military,naval}_lines` + `production.military_factories`
    /// + `decisions.timed_live` present (the Phase-3 primary channel), to prove the live path.
    fn snap_live() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": { "factories": {"civ": 120, "mil": 140, "dockyard": 18, "civ_from_states": 118},
                    "resources": { "available_derived": {"steel": 1000.0} } },
                "production": {
                    "military_factories": 137, "naval_factories": 16,
                    "military_lines": [
                        {"id": 349, "archetype": "infantry_equipment_1", "active_factories": 30, "amount": -1,
                         "speed": 200.0, "priority": 2, "produced": 5.0, "cost": 10.0}
                    ],
                    "naval_lines": [],
                    "construction_queue": [
                        {"line_id": 701, "template_id": 7, "active_factories": 12, "produced": 50.0, "cost": 200.0, "priority": 0}
                    ]
                },
                "decisions": {
                    "timed_live": [
                        {"decision": "war_economy_decision", "days": 42, "state": 64, "state_name": "Berlin"}
                    ]
                }
            } }
        }))
        .unwrap()
    }

    fn snap_production_a() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": { "factories": {"mil": 120, "dockyard": 18},
                    "resources": { "available_derived": {"steel": 1000.0} } },
                "production": {
                    "military_factories": 21, "naval_factories": 4,
                    "military_lines": [
                        {"id": "A_mil_01", "archetype": "infantry_equipment_1", "active_factories": 10,
                         "speed": 100.0, "priority": 0, "produced": 5.0, "cost": 10.0, "amount": -1},
                        {"id": "A_mil_02", "archetype": "medium_tank_chassis_1", "active_factories": 11,
                         "speed": 110.0, "priority": 1, "produced": 6.0, "cost": 12.0, "amount": -1}
                    ],
                    "naval_lines": [
                        {"id": "A_nav_01", "archetype": "ship_hull_light_1", "active_factories": 4,
                         "speed": 20.0, "priority": 2, "produced": 20.0, "cost": 100.0, "amount": 3}
                    ]
                }
            } }
        }))
        .unwrap()
    }

    fn snap_production_b_same_selected() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.13", "hour_index": 201,
            "countries": { "GER": {
                "economy": { "factories": {"mil": 121, "dockyard": 19},
                    "resources": { "available_derived": {"steel": 999.0} } },
                "production": {
                    "military_factories": 29, "naval_factories": 4,
                    "military_lines": [
                        {"id": "A_mil_01", "archetype": "infantry_equipment_1", "active_factories": 10,
                         "speed": 100.0, "priority": 0, "produced": 5.0, "cost": 10.0, "amount": -1},
                        {"id": "A_mil_02", "archetype": "medium_tank_chassis_1", "active_factories": 19,
                         "speed": 130.0, "priority": 1, "produced": 9.0, "cost": 12.0, "amount": -1}
                    ],
                    "naval_lines": [
                        {"id": "A_nav_01", "archetype": "ship_hull_light_1", "active_factories": 4,
                         "speed": 20.0, "priority": 2, "produced": 20.0, "cost": 100.0, "amount": 3}
                    ]
                }
            } }
        }))
        .unwrap()
    }

    fn snap_production_b_missing_selected() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.13", "hour_index": 201,
            "countries": { "GER": {
                "economy": { "factories": {"mil": 121, "dockyard": 19} },
                "production": {
                    "military_factories": 31, "naval_factories": 4,
                    "military_lines": [
                        {"id": "A_mil_01", "archetype": "infantry_equipment_1", "active_factories": 10,
                         "speed": 100.0, "priority": 0, "produced": 5.0, "cost": 10.0, "amount": -1},
                        {"id": "B_mil_03", "archetype": "fighter_equipment_0", "active_factories": 21,
                         "speed": 150.0, "priority": 2, "produced": 1.0, "cost": 20.0, "amount": -1}
                    ],
                    "naval_lines": [
                        {"id": "A_nav_01", "archetype": "ship_hull_light_1", "active_factories": 4,
                         "speed": 20.0, "priority": 2, "produced": 20.0, "cost": 100.0, "amount": 3}
                    ]
                }
            } }
        }))
        .unwrap()
    }

    fn snap_production_missing_first_identity() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": { "factories": {"mil": 120, "dockyard": 18} },
                "production": {
                    "military_factories": 21, "naval_factories": 0,
                    "military_lines": [
                        {"archetype": "infantry_equipment_1", "active_factories": 10,
                         "speed": 100.0, "priority": 0, "produced": 5.0, "cost": 10.0, "amount": -1},
                        {"id": "keyed_line", "archetype": "medium_tank_chassis_1", "active_factories": 11,
                         "speed": 110.0, "priority": 1, "produced": 6.0, "cost": 12.0, "amount": -1}
                    ],
                    "naval_lines": []
                }
            } }
        }))
        .unwrap()
    }

    fn snap_production_duplicate_ids() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": { "factories": {"mil": 120, "dockyard": 18} },
                "production": {
                    "military_factories": 31, "naval_factories": 0,
                    "military_lines": [
                        {"id": "dup_line", "archetype": "infantry_equipment_1", "active_factories": 10,
                         "speed": 100.0, "priority": 0, "produced": 5.0, "cost": 10.0, "amount": -1},
                        {"id": "dup_line", "archetype": "infantry_equipment_2", "active_factories": 11,
                         "speed": 110.0, "priority": 1, "produced": 6.0, "cost": 12.0, "amount": -1},
                        {"id": "unique_line", "archetype": "medium_tank_chassis_1", "active_factories": 12,
                         "speed": 120.0, "priority": 2, "produced": 7.0, "cost": 14.0, "amount": -1}
                    ],
                    "naval_lines": []
                }
            } }
        }))
        .unwrap()
    }

    fn snap_production_unknown_archetype() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": { "factories": {"mil": 120, "dockyard": 18} },
                "production": {
                    "military_factories": 31, "naval_factories": 0,
                    "military_lines": [
                        {"id": "known_infantry", "archetype": "infantry_equipment_1", "active_factories": 10,
                         "speed": 100.0, "priority": 0, "produced": 5.0, "cost": 10.0, "amount": -1},
                        {"id": "unknown_line", "archetype": "mystery_equipment_token", "active_factories": 11,
                         "speed": 110.0, "priority": 1, "produced": 6.0, "cost": 12.0, "amount": -1}
                    ],
                    "naval_lines": []
                }
            } }
        }))
        .unwrap()
    }

    fn snap_production_live_join_save_archetype() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": {
                    "factories": {"mil": 120, "dockyard": 18},
                    "_production_lines": [
                        {"kind": "military_lines", "line_id": 318, "archetype": "medium_tank_chassis_1",
                         "active_factories": 20, "speed": 90.0, "priority": 1, "produced": 1.0, "cost": 10.0},
                        {"kind": "naval_lines", "line_id": 401, "archetype": "ship_hull_light_1",
                         "active_factories": 4, "speed": 20.0, "priority": 2, "produced": 2.0, "cost": 100.0, "amount": 3}
                    ]
                },
                "production": {
                    "military_factories": 20, "naval_factories": 4,
                    "military_lines": [
                        {"id": 318, "active_factories": 20,
                         "speed": 90.0, "priority": 1, "produced": 1.0, "cost": 10.0}
                    ],
                    "naval_lines": [
                        {"id": 401, "active_factories": 4,
                         "speed": 20.0, "priority": 2, "produced": 2.0, "cost": 100.0, "amount": 3}
                    ]
                }
            } }
        }))
        .unwrap()
    }

    fn snap_production_duplicate_save_join() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": {
                    "factories": {"mil": 120, "dockyard": 18},
                    "_production_lines": [
                        {"kind": "military_lines", "line_id": 318, "archetype": "medium_tank_chassis_1"},
                        {"kind": "military_lines", "line_id": 318, "archetype": "infantry_equipment_1"}
                    ]
                },
                "production": {
                    "military_factories": 20, "naval_factories": 0,
                    "military_lines": [
                        {"id": 318, "active_factories": 20,
                         "speed": 90.0, "priority": 1, "produced": 1.0, "cost": 10.0}
                    ],
                    "naval_lines": []
                }
            } }
        }))
        .unwrap()
    }

    fn many_production_snap(count: usize, prefix: &str) -> snapshot::Snapshot {
        let rows: Vec<_> = (1..=count)
            .map(|idx| {
                serde_json::json!({
                    "id": format!("{prefix}_line_{idx:02}"),
                    "archetype": format!("{prefix}_equipment_{idx:02}"),
                    "active_factories": idx,
                    "speed": idx as f64,
                    "priority": idx % 3,
                    "produced": idx as f64,
                    "cost": 100.0,
                    "amount": -1
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": { "factories": {"mil": 120, "dockyard": 18} },
                "production": {
                    "military_factories": count,
                    "naval_factories": 0,
                    "military_lines": rows,
                    "naval_lines": []
                }
            } }
        }))
        .unwrap()
    }

    fn snap_construction_a() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": { "factories": {"civ": 120, "civ_from_states": 118} },
                "production": {
                    "construction_queue": [
                        {"line_id": "A_line_01", "template_id": 7, "active_factories": 10,
                         "produced": 50.0, "cost": 200.0, "priority": 0},
                        {"line_id": "A_line_02", "template_id": 8, "active_factories": 11,
                         "produced": 60.0, "cost": 200.0, "priority": 1}
                    ]
                }
            } }
        }))
        .unwrap()
    }

    fn snap_construction_b_same_selected() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.13", "hour_index": 201,
            "countries": { "GER": {
                "economy": { "factories": {"civ": 121, "civ_from_states": 119} },
                "production": {
                    "construction_queue": [
                        {"line_id": "A_line_01", "template_id": 7, "active_factories": 10,
                         "produced": 55.0, "cost": 200.0, "priority": 0},
                        {"line_id": "A_line_02", "template_id": 8, "active_factories": 19,
                         "produced": 99.0, "cost": 200.0, "priority": 1}
                    ]
                }
            } }
        }))
        .unwrap()
    }

    fn snap_construction_b_missing_selected() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.13", "hour_index": 201,
            "countries": { "GER": {
                "economy": { "factories": {"civ": 121, "civ_from_states": 119} },
                "production": {
                    "construction_queue": [
                        {"line_id": "A_line_01", "template_id": 7, "active_factories": 10,
                         "produced": 55.0, "cost": 200.0, "priority": 0},
                        {"line_id": "B_line_03", "template_id": 9, "active_factories": 21,
                         "produced": 20.0, "cost": 200.0, "priority": 2}
                    ]
                }
            } }
        }))
        .unwrap()
    }

    fn snap_construction_missing_first_identity() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": { "factories": {"civ": 120, "civ_from_states": 118} },
                "production": {
                    "construction_queue": [
                        {"template_id": 7, "active_factories": 10,
                         "produced": 50.0, "cost": 200.0, "priority": 0},
                        {"line_id": "keyed_line", "template_id": 8, "active_factories": 11,
                         "produced": 60.0, "cost": 200.0, "priority": 1}
                    ]
                }
            } }
        }))
        .unwrap()
    }

    fn many_construction_snap(count: usize, prefix: &str) -> snapshot::Snapshot {
        let rows: Vec<_> = (1..=count)
            .map(|idx| {
                serde_json::json!({
                    "line_id": format!("{prefix}_line_{idx:02}"),
                    "template_id": idx,
                    "active_factories": idx,
                    "produced": idx as f64,
                    "cost": 100.0,
                    "priority": idx % 3
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": { "factories": {"civ": 120, "civ_from_states": 118} },
                "production": { "construction_queue": rows }
            } }
        }))
        .unwrap()
    }

    fn rebind_ctx<'a>(snap: &'a snapshot::Snapshot) -> RebindContext<'a> {
        RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit: false,
        }
    }

    fn snap_decisions_a() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": {}, "production": {},
                "decisions": {
                    "timed_live": [
                        {"decision": "repeat_decision", "days": 42, "state": 64, "state_name": "Berlin"},
                        {"decision": "repeat_decision", "days": 8, "state": 75, "state_name": "Hamburg"}
                    ],
                    "status_present": { "active_timed_decision": 2 }
                }
            } }
        }))
        .unwrap()
    }

    fn snap_decisions_b_same_selected() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.13", "hour_index": 201,
            "countries": { "GER": {
                "economy": {}, "production": {},
                "decisions": {
                    "timed_live": [
                        {"decision": "repeat_decision", "days": 41, "state": 64, "state_name": "Berlin"},
                        {"decision": "repeat_decision", "days": 99, "state": 75, "state_name": "Hamburg"}
                    ],
                    "status_present": { "active_timed_decision": 2 }
                }
            } }
        }))
        .unwrap()
    }

    fn snap_decisions_b_missing_selected() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.13", "hour_index": 201,
            "countries": { "GER": {
                "economy": {}, "production": {},
                "decisions": {
                    "timed_live": [
                        {"decision": "repeat_decision", "days": 41, "state": 64, "state_name": "Berlin"}
                    ],
                    "status_present": { "active_timed_decision": 1 }
                }
            } }
        }))
        .unwrap()
    }

    fn snap_decisions_missing_first_identity() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": {}, "production": {},
                "decisions": {
                    "timed_live": [
                        {"days": 42, "state": 64},
                        {"decision": "repeat_decision", "days": 8, "state": 75}
                    ]
                }
            } }
        }))
        .unwrap()
    }

    fn many_decisions_snap(count: usize, prefix: &str) -> snapshot::Snapshot {
        let timed: Vec<_> = (1..=count)
            .map(|i| {
                serde_json::json!({
                    "decision": format!("{prefix}_decision_{i:02}"),
                    "days": i,
                    "state": i
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": {}, "production": {},
                "decisions": {
                    "timed_live": timed,
                    "status_present": { "active_timed_decision": count }
                }
            } }
        }))
        .unwrap()
    }

    fn click_decision_row(view: &mut DecisionsView, row: usize) -> InputOutcome {
        let viewport = decisions_grid_fallback(view.screen);
        view.on_input(&ViewInput::PointerDown {
            x: viewport.x + 4.0,
            y: viewport.y + DECISION_ROW_H * (row as f32) + 4.0,
            button: PointerButton::Left,
        })
    }

    fn decision_select_button_center(view: &DecisionsView, visible_row: usize) -> (f32, f32) {
        let viewport = decisions_grid_fallback(view.screen);
        let row_y = viewport.y + DECISION_ROW_H * visible_row as f32;
        let rect = decision_select_button_rect(viewport, row_y);
        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
    }

    fn wheel_decisions(view: &mut DecisionsView, dy: f32) -> InputOutcome {
        let viewport = decisions_grid_fallback(view.screen);
        view.on_input(&ViewInput::Wheel {
            x: viewport.x + 8.0,
            y: viewport.y + DECISION_ROW_H * 0.5,
            dy,
        })
    }

    fn decision_scrollbar(v: &DecisionsView) -> DecisionScrollbarGeometry {
        let viewport = decisions_grid_fallback(v.screen);
        decision_scrollbar_geometry(viewport, v.timed_keys.len(), v.timed_scroll_y)
            .expect("timed decisions list has scrollbar")
    }

    fn pointer_down_decision_thumb(v: &mut DecisionsView) -> InputOutcome {
        let geom = decision_scrollbar(v);
        v.on_input(&ViewInput::PointerDown {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.thumb.y + geom.thumb.h * 0.5,
            button: PointerButton::Left,
        })
    }

    fn click_production_line(view: &mut ProductionView, row: usize) -> InputOutcome {
        let viewport = production_grid_fallback(view.screen);
        view.on_input(&ViewInput::PointerDown {
            x: viewport.x + 122.0,
            y: viewport.y + PRODUCTION_ROW_H * (row as f32) + 96.0,
            button: PointerButton::Left,
        })
    }

    fn wheel_production(view: &mut ProductionView, dy: f32) -> InputOutcome {
        let viewport = production_grid_fallback(view.screen);
        view.on_input(&ViewInput::Wheel {
            x: viewport.x + 8.0,
            y: viewport.y + PRODUCTION_ROW_H * 0.5,
            dy,
        })
    }

    fn production_scrollbar(v: &ProductionView) -> ProductionScrollbarGeometry {
        let viewport = production_grid_fallback(v.screen);
        production_scrollbar_geometry(viewport, v.line_keys.len(), v.line_scroll_y)
            .expect("production list has scrollbar")
    }

    fn pointer_down_production_thumb(v: &mut ProductionView) -> InputOutcome {
        let geom = production_scrollbar(v);
        v.on_input(&ViewInput::PointerDown {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.thumb.y + geom.thumb.h * 0.5,
            button: PointerButton::Left,
        })
    }

    fn production_close_button_center(view: &ProductionView, visible_row: usize) -> (f32, f32) {
        let viewport = production_grid_fallback(view.screen);
        let row_y = viewport.y + PRODUCTION_ROW_H * visible_row as f32;
        let rect = production_line_close_button_rect(viewport, row_y);
        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
    }

    fn production_factory_button_centers(
        view: &ProductionView,
        visible_row: usize,
    ) -> [(f32, f32); 2] {
        let viewport = production_grid_fallback(view.screen);
        let row_y = viewport.y + PRODUCTION_ROW_H * visible_row as f32;
        let rects = production_factory_count_button_rects(viewport, row_y);
        rects.map(|rect| (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5))
    }

    fn production_add_line_button_centers(view: &ProductionView) -> Vec<(f32, f32)> {
        production_add_line_button_rects(view.screen)
            .into_iter()
            .map(|rect| (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5))
            .collect()
    }

    fn production_filter_button_center(
        view: &ProductionView,
        filter: ProductionPrimaryFilter,
    ) -> (f32, f32) {
        let rect = production_primary_filter_button_rects(view.screen)
            .into_iter()
            .find_map(|(candidate, rect)| (candidate == filter).then_some(rect))
            .expect("primary production filter exists");
        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
    }

    fn click_production_filter(
        view: &mut ProductionView,
        filter: ProductionPrimaryFilter,
    ) -> InputOutcome {
        let (x, y) = production_filter_button_center(view, filter);
        view.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        })
    }

    fn production_status_filter_button_centers(view: &ProductionView) -> Vec<(f32, f32)> {
        production_status_filter_button_rects(view.screen)
            .into_iter()
            .map(|(_, rect)| (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5))
            .collect()
    }

    fn production_priority_amount_button_centers(
        view: &ProductionView,
        visible_row: usize,
        kind: ProductionLineKind,
    ) -> Vec<(f32, f32)> {
        let viewport = production_grid_fallback(view.screen);
        let row_y = viewport.y + PRODUCTION_ROW_H * visible_row as f32;
        production_priority_button_rects(viewport, row_y)
            .into_iter()
            .chain(production_amount_button_rects(viewport, row_y, kind))
            .map(|rect| (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5))
            .collect()
    }

    fn production_line_mutation_control_centers(
        view: &ProductionView,
        visible_row: usize,
        kind: ProductionLineKind,
    ) -> Vec<(f32, f32)> {
        let viewport = production_grid_fallback(view.screen);
        let row_y = viewport.y + PRODUCTION_ROW_H * visible_row as f32;
        production_line_mutation_control_rects(viewport, row_y, kind)
            .into_iter()
            .map(|rect| (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5))
            .collect()
    }

    fn click_construction_row(view: &mut ConstructionView, row: usize) -> InputOutcome {
        let viewport = construction_grid_fallback(view.screen);
        view.on_input(&ViewInput::PointerDown {
            x: viewport.x + 4.0,
            y: viewport.y + CONSTRUCTION_ROW_H * (row as f32) + 4.0,
            button: PointerButton::Left,
        })
    }

    fn wheel_construction(view: &mut ConstructionView, dy: f32) -> InputOutcome {
        let viewport = construction_grid_fallback(view.screen);
        view.on_input(&ViewInput::Wheel {
            x: viewport.x + 8.0,
            y: viewport.y + CONSTRUCTION_ROW_H * 0.5,
            dy,
        })
    }

    fn construction_scrollbar(v: &ConstructionView) -> ConstructionScrollbarGeometry {
        let viewport = construction_grid_fallback(v.screen);
        construction_scrollbar_geometry(viewport, v.queue_keys.len(), v.queue_scroll_y)
            .expect("construction queue has scrollbar")
    }

    fn pointer_down_construction_thumb(v: &mut ConstructionView) -> InputOutcome {
        let geom = construction_scrollbar(v);
        v.on_input(&ViewInput::PointerDown {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.thumb.y + geom.thumb.h * 0.5,
            button: PointerButton::Left,
        })
    }

    fn construction_factory_count_centers(view: &ConstructionView) -> [(f32, f32); 2] {
        let rects = construction_factory_count_button_rects(view.screen);
        [
            (rects[0].x + rects[0].w * 0.5, rects[0].y + rects[0].h * 0.5),
            (rects[1].x + rects[1].w * 0.5, rects[1].y + rects[1].h * 0.5),
        ]
    }

    fn construction_temp_button_center(view: &ConstructionView) -> (f32, f32) {
        let rect = construction_temp_button_rect(view.screen);
        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
    }

    fn construction_queue_button_centers(
        view: &ConstructionView,
        visible_row: usize,
    ) -> Vec<(f32, f32)> {
        let viewport = construction_grid_fallback(view.screen);
        let row_y = viewport.y + CONSTRUCTION_ROW_H * visible_row as f32;
        construction_queue_button_rects(viewport, row_y)
            .into_iter()
            .map(|rect| (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5))
            .collect()
    }

    fn production_scroll_pair(d: &str) -> (i32, i32) {
        let rest = d
            .split(" scroll=")
            .nth(1)
            .expect("Production digest has scroll marker");
        let pair = rest
            .split(" visible=")
            .next()
            .expect("Production digest has visible marker");
        let (cur, max) = pair.split_once('/').expect("production scroll pair");
        (
            cur.parse().expect("scroll current is int"),
            max.parse().expect("scroll max is int"),
        )
    }

    fn visible_production_chunk(d: &str) -> &str {
        d.split(" visible=[")
            .nth(1)
            .and_then(|rest| rest.split("] mil=").next())
            .expect("Production digest has visible row chunk")
    }

    fn construction_scroll_pair(d: &str) -> (i32, i32) {
        let rest = d
            .split(" scroll=")
            .nth(1)
            .expect("Construction digest has scroll marker");
        let pair = rest
            .split(" visible=")
            .next()
            .expect("Construction digest has visible marker");
        let (cur, max) = pair.split_once('/').expect("construction scroll pair");
        (
            cur.parse().expect("scroll current is int"),
            max.parse().expect("scroll max is int"),
        )
    }

    fn visible_construction_chunk(d: &str) -> &str {
        d.split(" visible=[")
            .nth(1)
            .and_then(|rest| rest.split("] queue=").next())
            .expect("Construction digest has visible row chunk")
    }

    fn decisions_scroll_pair(d: &str) -> (i32, i32) {
        let rest = d
            .split(" scroll=")
            .nth(1)
            .expect("Decisions digest has scroll marker");
        let pair = rest
            .split(" visible=")
            .next()
            .expect("Decisions digest has visible marker");
        let (cur, max) = pair.split_once('/').expect("decisions scroll pair");
        (
            cur.parse().expect("scroll current is int"),
            max.parse().expect("scroll max is int"),
        )
    }

    fn visible_decisions_chunk(d: &str) -> &str {
        d.split(" visible=[")
            .nth(1)
            .and_then(|rest| rest.split("] timed=").next())
            .expect("Decisions digest has visible row chunk")
    }

    // -------- render-plan digest: scrub A→B→A byte-identical (the immutable-LRU rule) -----

    #[test]
    fn production_scrub_a_b_a_byte_identical() {
        let stage = stage_with(&["production.json"]);
        let (a, b) = (snap_save(), snap_live());
        let v = ProductionView::new();
        let da1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        let db = bound_digest(&v.bind(&ctx(&stage, &b)));
        assert_ne!(
            da1, db,
            "A (save) and B (live) must differ, else the scrub proves nothing"
        );
        let da2 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert_eq!(
            da1, da2,
            "scrub A→B→A: production carried a value from the B visit"
        );
    }

    #[test]
    fn production_line_row_click_selects_measured_live_id_key() {
        let stage = stage_with(&[]);
        let snap = snap_production_a();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        let default = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            default.contains("selected=none"),
            "pre-click production view should not claim a selection, got: {default}"
        );

        let out = click_production_line(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        assert!(out.commands.is_empty());

        let selected = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected=selected"),
            "row click must select a production line, got: {selected}"
        );
        assert!(
            selected.contains("selected_key=production:military:id:A_mil_02"),
            "selection must use measured live id identity, got: {selected}"
        );
        assert!(
            selected.contains("selected_row=r0<military>r0[key=production:military:id:A_mil_02;"),
            "selected row digest must point at the clicked backing military row, got: {selected}"
        );
        assert!(
            selected.contains("factories_used_txt=11(E)"),
            "clicked second row's factories must be visible in selected digest, got: {selected}"
        );
    }

    #[test]
    fn production_save_line_row_click_selects_measured_line_id_key() {
        let stage = stage_with(&[]);
        let snap = snap_save();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        let out = click_production_line(&mut view, 0);
        assert!(out.consumed && out.needs_redraw);
        let selected = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected_key=production:military:line:318"),
            "save fallback selection must use measured line_id identity, got: {selected}"
        );
        assert!(
            selected.contains("factories_used_txt=25(E)"),
            "save-backed selected row should still bind its measured factories, got: {selected}"
        );
    }

    #[test]
    fn production_naval_line_row_click_selects_measured_live_id_key() {
        let stage = stage_with(&[]);
        let snap = snap_production_a();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        let out = click_production_line(&mut view, 2);
        assert!(out.consumed && out.needs_redraw);
        let selected = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected_key=production:naval:id:A_nav_01"),
            "mixed production list selection must preserve naval line identity, got: {selected}"
        );
        assert!(
            selected.contains("selected_row=r0<naval>r0[key=production:naval:id:A_nav_01;"),
            "selected row digest must point at the clicked backing naval row, got: {selected}"
        );
        assert!(
            selected.contains("amount=3(E)"),
            "selected naval row should bind its measured amount cell, got: {selected}"
        );
    }

    #[test]
    fn production_primary_filter_toggle_filters_rendered_rows_without_commands() {
        let stage = stage_with(&[]);
        let snap = snap_production_a();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("filters=equipment=1,armor=1,air=1"));
        assert!(visible_production_chunk(&before).contains("medium_tank_chassis_1"));

        let out = click_production_filter(&mut view, ProductionPrimaryFilter::Armor);
        assert!(out.consumed && out.needs_redraw);
        assert!(out.commands.is_empty());

        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let visible = visible_production_chunk(&after);
        assert!(after.contains("filters=equipment=1,armor=0,air=1"));
        assert!(
            !visible.contains("medium_tank_chassis_1"),
            "armor filter off must hide positively classified armor rows: {after}"
        );
        assert!(visible.contains("infantry_equipment_1"));
        assert!(visible.contains("ship_hull_light_1"));
    }

    #[test]
    fn production_primary_filter_clicks_drive_filtered_row_selection_after_rebind() {
        let stage = stage_with(&[]);
        let snap = snap_production_a();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        click_production_filter(&mut view, ProductionPrimaryFilter::Armor);
        view.rebind(&rebind_ctx(&snap));
        let out = click_production_line(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);

        let selected = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected_key=production:naval:id:A_nav_01"),
            "after the armor row is filtered out, visible row 1 must select the naval backing row: {selected}"
        );
    }

    #[test]
    fn production_status_filter_buttons_are_replay_disabled_noops() {
        let stage = stage_with(&[]);
        let snap = snap_production_a();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        let select = click_production_line(&mut view, 1);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            before.contains("selected_key=production:military:id:A_mil_02"),
            "test setup must select a non-overlapped row before status-filter clicks: {before}"
        );
        let before_selected = view.selected_line_key.clone();
        let before_scroll = production_scroll_pair(&before);

        for (x, y) in production_status_filter_button_centers(&view) {
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                down.consumed && !down.needs_redraw,
                "production status filters are unsupported replay-local filters and must be disabled no-ops"
            );
            assert!(down.commands.is_empty());
            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                up.consumed && !up.needs_redraw,
                "production status filter pointer-up must also be a disabled no-op"
            );
            assert!(up.commands.is_empty());
        }

        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            view.selected_line_key, before_selected,
            "status-filter no-op must not change selected production line"
        );
        assert_eq!(production_scroll_pair(&after), before_scroll);
        assert_eq!(
            after, before,
            "status-filter no-op must not change the production bind digest"
        );
    }

    #[test]
    fn production_primary_filter_keeps_unknown_categories_visible() {
        let stage = stage_with(&[]);
        let snap = snap_production_unknown_archetype();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        click_production_filter(&mut view, ProductionPrimaryFilter::Equipment);
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let visible = visible_production_chunk(&after);
        assert!(
            !visible.contains("known_infantry"),
            "equipment filter off must hide catalog-classified infantry rows: {after}"
        );
        assert!(
            visible.contains("unknown_line") && visible.contains("mystery_equipment_token"),
            "unknown catalog tokens must remain visible instead of being guessed into a category: {after}"
        );
    }

    #[test]
    fn production_primary_filter_uses_unique_save_join_for_live_rows_without_archetype() {
        let stage = stage_with(&[]);
        let snap = snap_production_live_join_save_archetype();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            before.contains("name=—"),
            "live row still honestly lacks a rendered archetype token: {before}"
        );

        click_production_filter(&mut view, ProductionPrimaryFilter::Armor);
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            !visible_production_chunk(&after).contains("production:military:id:318"),
            "unique save line_id join must classify the live row as armor and hide it: {after}"
        );
    }

    #[test]
    fn production_primary_filter_duplicate_save_join_fails_open() {
        let stage = stage_with(&[]);
        let snap = snap_production_duplicate_save_join();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        click_production_filter(&mut view, ProductionPrimaryFilter::Armor);
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            visible_production_chunk(&after).contains("production:military:id:318"),
            "duplicate save line_id matches must not guess a category or hide the live row: {after}"
        );
    }

    #[test]
    fn production_selected_line_scrub_a_b_a_rebuilds_current_hour_row() {
        let stage = stage_with(&[]);
        let a = snap_production_a();
        let b = snap_production_b_same_selected();
        let mut view = ProductionView::new();

        view.rebind(&rebind_ctx(&a));
        let out = click_production_line(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert!(a1.contains("selected_key=production:military:id:A_mil_02"));
        assert!(a1.contains("factories_used_txt=11(E)"));

        view.rebind(&rebind_ctx(&b));
        let b_digest = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            b_digest.contains("selected_key=production:military:id:A_mil_02"),
            "selected key must survive into B, got: {b_digest}"
        );
        assert!(
            b_digest.contains("selected_row=r0<military>r0[key=production:military:id:A_mil_02;"),
            "B selected-row digest must still point at the selected backing row, got: {b_digest}"
        );
        assert!(
            b_digest.contains("factories_used_txt=19(E)"),
            "B selected row must rebuild from B's current row, got: {b_digest}"
        );

        view.rebind(&rebind_ctx(&a));
        let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "selected production line scrub A->B->A must be byte-identical"
        );
    }

    #[test]
    fn production_missing_selected_line_stays_missing_without_row_fallback() {
        let stage = stage_with(&[]);
        let a = snap_production_a();
        let b = snap_production_b_missing_selected();
        let mut view = ProductionView::new();

        view.rebind(&rebind_ctx(&a));
        let out = click_production_line(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert!(a1.contains("selected_key=production:military:id:A_mil_02"));

        view.rebind(&rebind_ctx(&b));
        let missing = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            missing.contains("selected=missing_selected"),
            "missing selected production line must be explicit, got: {missing}"
        );
        assert!(
            missing.contains("selected_key=production:military:id:A_mil_02"),
            "the missing selected key must be retained for the badge, got: {missing}"
        );
        assert!(
            missing.contains("selected_row=none"),
            "missing selected row must not fall back to another row, got: {missing}"
        );
        assert!(
            !missing.contains("selected_row=r0<military>r0[key=production:military:id:A_mil_01;")
                && !missing
                    .contains("selected_row=r0<military>r0[key=production:military:id:B_mil_03;"),
            "missing selected row must not highlight another production row, got: {missing}"
        );

        view.rebind(&rebind_ctx(&a));
        let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "returning to A must restore the selected production row exactly"
        );
    }

    #[test]
    fn production_unkeyed_duplicate_and_outside_row_clicks_are_inert() {
        let stage = stage_with(&[]);
        let snap = snap_production_missing_first_identity();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        let unkeyed = click_production_line(&mut view, 0);
        assert!(
            !unkeyed.consumed && !unkeyed.needs_redraw && unkeyed.commands.is_empty(),
            "row without measured production identity must not be selected by index"
        );
        assert!(view.selected_line_key.is_none());

        let viewport = production_grid_fallback(view.screen);
        let outside = view.on_input(&ViewInput::PointerDown {
            x: viewport.x,
            y: viewport.y + viewport.h + PRODUCTION_ROW_H,
            button: PointerButton::Left,
        });
        assert!(
            !outside.consumed && !outside.needs_redraw && outside.commands.is_empty(),
            "outside production-row click must be inert"
        );

        let keyed = click_production_line(&mut view, 1);
        assert!(keyed.consumed && keyed.needs_redraw);
        assert_eq!(
            view.selected_line_key.as_deref(),
            Some("production:military:id:keyed_line")
        );

        let dup_snap = snap_production_duplicate_ids();
        let mut dup_view = ProductionView::new();
        dup_view.rebind(&rebind_ctx(&dup_snap));
        let duplicate = click_production_line(&mut dup_view, 0);
        assert!(
            !duplicate.consumed && !duplicate.needs_redraw && duplicate.commands.is_empty(),
            "duplicate measured production ids must be unselectable"
        );
        let duplicate_second = click_production_line(&mut dup_view, 1);
        assert!(
            !duplicate_second.consumed
                && !duplicate_second.needs_redraw
                && duplicate_second.commands.is_empty(),
            "all rows sharing a duplicate production id must be unselectable"
        );
        let unique = click_production_line(&mut dup_view, 2);
        assert!(unique.consumed && unique.needs_redraw);
        let selected = bound_digest(&dup_view.bind(&ctx(&stage, &dup_snap)));
        assert!(
            selected.contains("selected_key=production:military:id:unique_line"),
            "unique row after duplicate rows must remain selectable, got: {selected}"
        );
    }

    #[test]
    fn production_line_wheel_scroll_changes_visible_rows_and_clamps() {
        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(production_scroll_pair(&before).0, 0);
        assert!(
            visible_production_chunk(&before)
                .contains("key=production:military:id:A_line_01;name=A_equipment_01(D);"),
            "initial visible rows must start at line 1: {before}"
        );

        let out = wheel_production(&mut view, -3.0);
        assert!(out.consumed && out.needs_redraw);
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(production_scroll_pair(&after).0, 333);
        assert!(
            visible_production_chunk(&after)
                .contains("key=production:military:id:A_line_04;name=A_equipment_04(D);"),
            "after three-row wheel scroll, visible rows must advance: {after}"
        );
        assert!(
            !visible_production_chunk(&after).contains("key=production:military:id:A_line_01;"),
            "scrolled visible slice must not keep top rows: {after}"
        );

        let out = wheel_production(&mut view, -500.0);
        assert!(out.consumed && out.needs_redraw);
        let clamped = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let (cur, max) = production_scroll_pair(&clamped);
        assert_eq!(cur, max);

        let out = wheel_production(&mut view, 500.0);
        assert!(out.consumed && out.needs_redraw);
        let reset = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(production_scroll_pair(&reset).0, 0);
    }

    #[test]
    fn production_scrolled_line_click_selects_correct_backing_key() {
        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        wheel_production(&mut view, -5.0);
        let out = click_production_line(&mut view, 0);
        assert!(out.consumed && out.needs_redraw);
        let d = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("selected_key=production:military:id:A_line_06"),
            "after five-row scroll, visible row 0 must select backing production line 6, got:\n{d}"
        );
        assert!(
            d.contains("selected_row=r0<military>r0[key=production:military:id:A_line_06;"),
            "selected digest must point at backing production line 6, got:\n{d}"
        );
    }

    #[test]
    fn production_scroll_survives_scrub_a_b_a_without_b_rows() {
        let stage = stage_with(&[]);
        let a = many_production_snap(40, "A");
        let b = many_production_snap(45, "B");
        let mut view = ProductionView::new();

        view.rebind(&rebind_ctx(&a));
        wheel_production(&mut view, -5.0);
        click_production_line(&mut view, 0);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert!(a1.contains("selected_key=production:military:id:A_line_06"));
        assert!(
            visible_production_chunk(&a1).contains("key=production:military:id:A_line_06;"),
            "A visible chunk should include selected row after scroll: {a1}"
        );

        view.rebind(&rebind_ctx(&b));
        let b_digest = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            b_digest.contains("selected=missing_selected"),
            "B uses different measured production ids, so A's selection must fail closed: {b_digest}"
        );
        assert_eq!(
            production_scroll_pair(&b_digest).0,
            production_scroll_pair(&a1).0
        );
        assert!(
            visible_production_chunk(&b_digest)
                .contains("key=production:military:id:B_line_06;name=B_equipment_06(D);"),
            "B visible chunk must rebuild from B rows at the retained scroll offset: {b_digest}"
        );
        assert!(
            !visible_production_chunk(&b_digest).contains("key=production:military:id:A_line_06;"),
            "B visible chunk must not carry A rows: {b_digest}"
        );

        view.rebind(&rebind_ctx(&a));
        let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "production scroll + selection must be byte-identical across A->B->A"
        );
    }

    #[test]
    fn production_wheel_outside_grid_is_inert() {
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        let viewport = production_grid_fallback(view.screen);

        let out = view.on_input(&ViewInput::Wheel {
            x: viewport.x + viewport.w + 20.0,
            y: viewport.y + PRODUCTION_ROW_H,
            dy: -5.0,
        });
        assert!(
            !out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "wheel outside the production line grid must be inert"
        );
        assert_eq!(view.line_scroll_y, 0.0);
    }

    #[test]
    fn production_scrollbar_drag_changes_visible_rows_and_clamps() {
        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(production_scroll_pair(&before).0, 0);

        let down = pointer_down_production_thumb(&mut view);
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "thumb pointer-down should consume to capture drag without changing scroll"
        );
        let geom = production_scrollbar(&view);
        let drag = view.on_input(&ViewInput::PointerMove {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.track.y + geom.track.h + 500.0,
            drag: true,
        });
        assert!(drag.consumed && drag.needs_redraw);
        let bottom = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let (cur, max) = production_scroll_pair(&bottom);
        assert_eq!(cur, max, "dragging below the track should clamp to max");
        assert!(
            !visible_production_chunk(&bottom).contains("key=production:military:id:A_line_01;"),
            "bottom-dragged visible chunk must not keep top rows: {bottom}"
        );

        let reset = view.on_input(&ViewInput::PointerMove {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.track.y - 500.0,
            drag: true,
        });
        assert!(reset.consumed && reset.needs_redraw);
        let top = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(production_scroll_pair(&top).0, 0);
        assert!(
            visible_production_chunk(&top).contains("key=production:military:id:A_line_01;"),
            "dragging above the track should clamp back to top rows: {top}"
        );
    }

    #[test]
    fn production_scrollbar_drag_capture_continues_outside_and_clears_on_up() {
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        let geom = production_scrollbar(&view);

        let down = pointer_down_production_thumb(&mut view);
        assert!(down.consumed && !down.needs_redraw);

        let drag = view.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 300.0,
            y: geom.track.y + geom.track.h,
            drag: true,
        });
        assert!(
            drag.consumed && drag.needs_redraw,
            "captured drag should continue even when x leaves the production list"
        );
        assert!(view.line_scroll_y > 0.0);

        let up = view.on_input(&ViewInput::PointerUp {
            x: geom.track.x - 300.0,
            y: geom.track.y + geom.track.h,
            button: PointerButton::Left,
        });
        assert!(
            up.consumed && !up.needs_redraw,
            "pointer-up should consume solely to clear scrollbar capture"
        );
        let ignored = view.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 300.0,
            y: geom.track.y,
            drag: true,
        });
        assert!(
            !ignored.consumed && !ignored.needs_redraw,
            "pointer move after thumb release must not keep stale production scrollbar capture"
        );
    }

    #[test]
    fn production_scrollbar_track_click_does_not_select_row() {
        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let geom = production_scrollbar(&view);
        let track_y = (geom.thumb.y + geom.thumb.h + 10.0).min(geom.track.y + geom.track.h - 1.0);
        let out = view.on_input(&ViewInput::PointerDown {
            x: geom.track.x + geom.track.w * 0.5,
            y: track_y,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "clicking the production scrollbar track outside the thumb should be an inert consumed UI hit"
        );
        assert!(
            view.selected_line_key.is_none(),
            "production scrollbar track click must not select an underlying line row"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "track click must not alter selected-row digest, got: {after}"
        );
    }

    #[test]
    fn production_line_close_button_is_replay_disabled_noop() {
        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        wheel_production(&mut view, -4.0);
        let select = click_production_line(&mut view, 0);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected_key=production:military:id:A_line_05"));
        let before_selected = view.selected_line_key.clone();
        let before_scroll = production_scroll_pair(&before);

        let (x, y) = production_close_button_center(&view, 0);
        let down = view.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "production close-line pointer-down must be a replay-disabled no-op"
        );
        let up = view.on_input(&ViewInput::PointerUp {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            up.consumed && !up.needs_redraw && up.commands.is_empty(),
            "production close-line pointer-up must be a replay-disabled no-op"
        );

        assert_eq!(
            view.selected_line_key, before_selected,
            "close-line no-op must not change selected production line"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(production_scroll_pair(&after), before_scroll);
        assert_eq!(
            before, after,
            "close-line no-op must not change scroll, selection, visible rows, or digest"
        );
    }

    #[test]
    fn production_factory_count_buttons_are_replay_disabled_noops() {
        let Some(gui) = load_gui(PRODUCTION_FILE) else {
            eprintln!("[production] skipped factory-button test: game_assets/gui not extracted");
            return;
        };
        for name in PRODUCTION_FACTORY_COUNT_BUTTON_NAMES {
            assert!(
                find_raw_node(&gui.windows, name).is_some(),
                "real production GUI must expose {name}"
            );
        }

        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        wheel_production(&mut view, -4.0);
        let select = click_production_line(&mut view, 0);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected_key=production:military:id:A_line_05"));
        let before_selected = view.selected_line_key.clone();
        let before_scroll = production_scroll_pair(&before);

        for (x, y) in production_factory_button_centers(&view, 0) {
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                down.consumed && !down.needs_redraw && down.commands.is_empty(),
                "production factory add/sub pointer-down must be a replay-disabled no-op"
            );
            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                up.consumed && !up.needs_redraw && up.commands.is_empty(),
                "production factory add/sub pointer-up must be a replay-disabled no-op"
            );
        }

        assert_eq!(
            view.selected_line_key, before_selected,
            "factory add/sub no-op must not change selected production line"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(production_scroll_pair(&after), before_scroll);
        assert_eq!(
            before, after,
            "factory add/sub no-op must not change scroll, selection, visible rows, or digest"
        );
    }

    #[test]
    fn production_add_line_buttons_are_replay_disabled_noops() {
        let Some(gui) = load_gui(PRODUCTION_FILE) else {
            eprintln!("[production] skipped add-line button test: game_assets/gui not extracted");
            return;
        };
        for name in PRODUCTION_ADD_LINE_BUTTON_NAMES {
            assert!(
                find_raw_node(&gui.windows, name).is_some(),
                "real production GUI must expose {name}"
            );
        }

        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        wheel_production(&mut view, -4.0);
        let select = click_production_line(&mut view, 0);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected_key=production:military:id:A_line_05"));
        let before_selected = view.selected_line_key.clone();
        let before_scroll = production_scroll_pair(&before);

        for (x, y) in production_add_line_button_centers(&view) {
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                down.consumed && !down.needs_redraw && down.commands.is_empty(),
                "production add-line pointer-down must be a replay-disabled no-op"
            );
            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                up.consumed && !up.needs_redraw && up.commands.is_empty(),
                "production add-line pointer-up must be a replay-disabled no-op"
            );
        }

        assert_eq!(
            view.selected_line_key, before_selected,
            "add-line no-op must not change selected production line"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(production_scroll_pair(&after), before_scroll);
        assert_eq!(
            before, after,
            "add-line no-op must not change scroll, selection, visible rows, or digest"
        );
    }

    #[test]
    fn production_add_line_buttons_do_not_claim_neighboring_empty_space() {
        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        wheel_production(&mut view, -4.0);
        click_production_line(&mut view, 0);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));

        let rects = production_add_line_button_rects(view.screen);
        let x = (rects[2].x + rects[2].w + rects[3].x) * 0.5;
        let y = rects[2].y + rects[2].h * 0.5;
        let out = view.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            !out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "empty space between production add-line button groups must stay unclaimed"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "empty add-line-area space must not mutate production view state"
        );
    }

    #[test]
    fn production_priority_amount_buttons_are_replay_disabled_noops() {
        let Some(gui) = load_gui(PRODUCTION_FILE) else {
            eprintln!(
                "[production] skipped priority/amount button test: game_assets/gui not extracted"
            );
            return;
        };
        for name in PRODUCTION_PRIORITY_AMOUNT_BUTTON_NAMES {
            assert!(
                find_raw_node(&gui.windows, name).is_some(),
                "real production GUI must expose {name}"
            );
        }

        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        wheel_production(&mut view, -4.0);
        let select = click_production_line(&mut view, 0);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected_key=production:military:id:A_line_05"));
        let before_selected = view.selected_line_key.clone();
        let before_scroll = production_scroll_pair(&before);

        for (x, y) in
            production_priority_amount_button_centers(&view, 0, ProductionLineKind::Military)
        {
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                down.consumed && !down.needs_redraw && down.commands.is_empty(),
                "production priority/amount pointer-down must be a replay-disabled no-op"
            );
            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                up.consumed && !up.needs_redraw && up.commands.is_empty(),
                "production priority/amount pointer-up must be a replay-disabled no-op"
            );
        }

        assert_eq!(
            view.selected_line_key, before_selected,
            "priority/amount no-op must not change selected production line"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(production_scroll_pair(&after), before_scroll);
        assert_eq!(
            before, after,
            "priority/amount no-op must not change scroll, selection, visible rows, or digest"
        );
    }

    #[test]
    fn production_naval_amount_buttons_are_replay_disabled_noops() {
        let stage = stage_with(&[]);
        let snap = snap_production_a();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        let select = click_production_line(&mut view, 2);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected_key=production:naval:id:A_nav_01"));
        let before_selected = view.selected_line_key.clone();

        for (x, y) in production_priority_amount_button_centers(&view, 2, ProductionLineKind::Naval)
        {
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                down.consumed && !down.needs_redraw && down.commands.is_empty(),
                "naval priority/amount pointer-down must be a replay-disabled no-op"
            );
            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                up.consumed && !up.needs_redraw && up.commands.is_empty(),
                "naval priority/amount pointer-up must be a replay-disabled no-op"
            );
        }

        assert_eq!(
            view.selected_line_key, before_selected,
            "naval priority/amount no-op must not change selected production line"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "naval priority/amount no-op must not change selection, visible rows, or digest"
        );
    }

    #[test]
    fn production_line_mutation_controls_are_replay_disabled_noops() {
        let Some(gui) = load_gui(PRODUCTION_FILE) else {
            eprintln!(
                "[production] skipped line mutation-control no-op test: game_assets/gui not extracted"
            );
            return;
        };
        let military = find_raw_window(&gui.windows, "production_military_line_entry")
            .expect("production_military_line_entry template exists");
        let naval = find_raw_window(&gui.windows, "production_naval_line_entry")
            .expect("production_naval_line_entry template exists");
        for (kind, name) in PRODUCTION_LINE_MUTATION_BUTTON_NAMES {
            let template = match kind {
                ProductionLineKind::Military => military,
                ProductionLineKind::Naval => naval,
            };
            assert!(
                find_raw_child(template, name).is_some(),
                "real production {:?} template must expose {name}",
                kind
            );
        }

        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));

        wheel_production(&mut view, -4.0);
        let select = click_production_line(&mut view, 0);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected_key=production:military:id:A_line_05"));
        let before_selected = view.selected_line_key.clone();
        let before_scroll = production_scroll_pair(&before);

        for (x, y) in
            production_line_mutation_control_centers(&view, 0, ProductionLineKind::Military)
        {
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                down.consumed && !down.needs_redraw && down.commands.is_empty(),
                "military production line mutation control pointer-down must be a replay-disabled no-op"
            );
            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                up.consumed && !up.needs_redraw && up.commands.is_empty(),
                "military production line mutation control pointer-up must be a replay-disabled no-op"
            );
        }

        assert_eq!(
            view.selected_line_key, before_selected,
            "military line-control no-op must not change selected production line"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(production_scroll_pair(&after), before_scroll);
        assert_eq!(
            before, after,
            "military line-control no-op must not change scroll, selection, visible rows, or digest"
        );

        let snap = snap_production_a();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        let select = click_production_line(&mut view, 2);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected_key=production:naval:id:A_nav_01"));
        let before_selected = view.selected_line_key.clone();

        for (x, y) in production_line_mutation_control_centers(&view, 2, ProductionLineKind::Naval)
        {
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                down.consumed && !down.needs_redraw && down.commands.is_empty(),
                "naval production line mutation control pointer-down must be a replay-disabled no-op"
            );
            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                up.consumed && !up.needs_redraw && up.commands.is_empty(),
                "naval production line mutation control pointer-up must be a replay-disabled no-op"
            );
        }

        assert_eq!(
            view.selected_line_key, before_selected,
            "naval line-control no-op must not change selected production line"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "naval line-control no-op must not change selection, visible rows, or digest"
        );
    }

    #[test]
    fn production_line_mutation_controls_do_not_claim_neighboring_row_clicks() {
        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        wheel_production(&mut view, -4.0);

        let viewport = production_grid_fallback(view.screen);
        let equipment = production_line_mutation_control_rects(
            viewport,
            viewport.y,
            ProductionLineKind::Military,
        )[2];
        let out = view.on_input(&ViewInput::PointerDown {
            x: equipment.x + equipment.w + 24.0,
            y: equipment.y + equipment.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && out.needs_redraw && out.commands.is_empty(),
            "clicks beside equipment drag/drop controls must still select the visible backing row"
        );
        let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            digest.contains("selected_key=production:military:id:A_line_05"),
            "neighboring equipment-area row click should select the scrolled row, got: {digest}"
        );

        let snap = snap_production_a();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        let viewport = production_grid_fallback(view.screen);
        let row_y = viewport.y + PRODUCTION_ROW_H * 2.0;
        let name_list =
            production_line_mutation_control_rects(viewport, row_y, ProductionLineKind::Naval)[6];
        let out = view.on_input(&ViewInput::PointerDown {
            x: name_list.x + name_list.w + 12.0,
            y: name_list.y + name_list.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && out.needs_redraw && out.commands.is_empty(),
            "clicks beside naval name-list controls must still select the visible backing row"
        );
        let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            digest.contains("selected_key=production:naval:id:A_nav_01"),
            "neighboring naval name-list row click should select the row, got: {digest}"
        );
    }

    #[test]
    fn production_line_close_button_does_not_claim_neighboring_row_clicks() {
        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        wheel_production(&mut view, -4.0);

        let viewport = production_grid_fallback(view.screen);
        let close = production_line_close_button_rect(viewport, viewport.y);
        let out = view.on_input(&ViewInput::PointerDown {
            x: close.x - 55.0,
            y: close.y + close.h + 18.0,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && out.needs_redraw && out.commands.is_empty(),
            "neighboring row body clicks must still select the visible backing row"
        );
        let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            digest.contains("selected_key=production:military:id:A_line_05"),
            "neighboring row click should select the scrolled row, got: {digest}"
        );
    }

    #[test]
    fn production_factory_count_buttons_do_not_claim_neighboring_row_clicks() {
        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        wheel_production(&mut view, -4.0);

        let viewport = production_grid_fallback(view.screen);
        let buttons = production_factory_count_button_rects(viewport, viewport.y);
        let out = view.on_input(&ViewInput::PointerDown {
            x: buttons[0].x - 20.0,
            y: buttons[0].y + buttons[0].h + 18.0,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && out.needs_redraw && out.commands.is_empty(),
            "clicks beside factory add/sub buttons must still select the visible backing row"
        );
        let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            digest.contains("selected_key=production:military:id:A_line_05"),
            "neighboring row click should select the scrolled row, got: {digest}"
        );
    }

    #[test]
    fn production_priority_amount_buttons_do_not_claim_neighboring_row_clicks() {
        let stage = stage_with(&[]);
        let snap = many_production_snap(40, "A");

        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        wheel_production(&mut view, -4.0);
        let viewport = production_grid_fallback(view.screen);
        let priority = production_priority_button_rects(viewport, viewport.y)[0];
        let out = view.on_input(&ViewInput::PointerDown {
            x: priority.x + priority.w + 110.0,
            y: priority.y + priority.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && out.needs_redraw && out.commands.is_empty(),
            "clicks beside priority buttons must still select the visible backing row"
        );
        let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            digest.contains("selected_key=production:military:id:A_line_05"),
            "neighboring priority-area row click should select the scrolled row, got: {digest}"
        );

        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        wheel_production(&mut view, -4.0);
        let viewport = production_grid_fallback(view.screen);
        let amount =
            production_amount_button_rects(viewport, viewport.y, ProductionLineKind::Military)[0];
        let out = view.on_input(&ViewInput::PointerDown {
            x: amount.x - 20.0,
            y: amount.y + amount.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && out.needs_redraw && out.commands.is_empty(),
            "clicks beside amount buttons must still select the visible backing row"
        );
        let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            digest.contains("selected_key=production:military:id:A_line_05"),
            "neighboring amount-area row click should select the scrolled row, got: {digest}"
        );

        let snap = snap_production_a();
        let mut view = ProductionView::new();
        view.rebind(&rebind_ctx(&snap));
        let viewport = production_grid_fallback(view.screen);
        let row_y = viewport.y + PRODUCTION_ROW_H * 2.0;
        let amount = production_amount_button_rects(viewport, row_y, ProductionLineKind::Naval)[0];
        let out = view.on_input(&ViewInput::PointerDown {
            x: amount.x - 20.0,
            y: amount.y + amount.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && out.needs_redraw && out.commands.is_empty(),
            "clicks beside naval amount buttons must still select the visible backing row"
        );
        let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            digest.contains("selected_key=production:naval:id:A_nav_01"),
            "neighboring naval amount-area row click should select the naval row, got: {digest}"
        );
    }

    #[test]
    fn construction_scrub_a_b_a_byte_identical() {
        let stage = stage_with(&["construction.json"]);
        let (a, b) = (snap_save(), snap_live());
        let v = ConstructionView::new();
        let da1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        let db = bound_digest(&v.bind(&ctx(&stage, &b)));
        assert_ne!(da1, db);
        let da2 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert_eq!(
            da1, da2,
            "scrub A→B→A: construction carried a value from the B visit"
        );
    }

    #[test]
    fn construction_queue_row_click_selects_measured_line_key() {
        let stage = stage_with(&[]);
        let snap = snap_construction_a();
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));

        let default = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            default.contains("selected=none"),
            "pre-click construction view should not claim a selection, got: {default}"
        );

        let out = click_construction_row(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        assert!(out.commands.is_empty());

        let selected = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected=selected"),
            "row click must select a construction queue row, got: {selected}"
        );
        assert!(
            selected.contains("selected_key=construction:line:A_line_02"),
            "selection must use measured live line_id identity, got: {selected}"
        );
        assert!(
            selected.contains("selected_row=r0[key=construction:line:A_line_02;"),
            "selected row digest must point at the clicked backing row, got: {selected}"
        );
        assert!(
            selected.contains("factories=11(E)"),
            "clicked second row's factories must be visible in selected digest, got: {selected}"
        );
    }

    #[test]
    fn construction_save_queue_row_click_selects_measured_template_state_created_key() {
        let stage = stage_with(&[]);
        let snap = snap_save();
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));

        let out = click_construction_row(&mut view, 0);
        assert!(out.consumed && out.needs_redraw);
        let selected = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains(
                "selected_key=construction:template:arms_factory:state:75:created:1940.8.3.13"
            ),
            "save fallback selection must use measured template/state/created identity, got: {selected}"
        );
        assert!(
            selected.contains("location=#75(D)"),
            "save-backed selected row should still bind its measured location, got: {selected}"
        );
    }

    #[test]
    fn construction_selected_row_scrub_a_b_a_rebuilds_current_hour_row() {
        let stage = stage_with(&[]);
        let a = snap_construction_a();
        let b = snap_construction_b_same_selected();
        let mut view = ConstructionView::new();

        view.rebind(&rebind_ctx(&a));
        let out = click_construction_row(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert!(a1.contains("selected_key=construction:line:A_line_02"));
        assert!(a1.contains("factories=11(E)"));

        view.rebind(&rebind_ctx(&b));
        let b_digest = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            b_digest.contains("selected_key=construction:line:A_line_02"),
            "selected key must survive into B, got: {b_digest}"
        );
        assert!(
            b_digest.contains("selected_row=r0[key=construction:line:A_line_02;"),
            "B selected-row digest must still point at the selected backing row, got: {b_digest}"
        );
        assert!(
            b_digest.contains("factories=19(E)"),
            "B selected row must rebuild from B's current row, got: {b_digest}"
        );

        view.rebind(&rebind_ctx(&a));
        let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "selected construction row scrub A->B->A must be byte-identical"
        );
    }

    #[test]
    fn construction_missing_selected_row_stays_selected_missing_without_row_fallback() {
        let stage = stage_with(&[]);
        let a = snap_construction_a();
        let b = snap_construction_b_missing_selected();
        let mut view = ConstructionView::new();

        view.rebind(&rebind_ctx(&a));
        let out = click_construction_row(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));

        view.rebind(&rebind_ctx(&b));
        let missing = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            missing.contains("selected=missing_selected"),
            "missing selected construction row must be explicit, got: {missing}"
        );
        assert!(
            missing.contains("selected_key=construction:line:A_line_02"),
            "missing digest must retain the requested key, got: {missing}"
        );
        assert!(
            missing.contains("selected_row=none"),
            "missing selected row must not fall back to another row, got: {missing}"
        );
        assert!(
            !missing.contains("selected_row=r0[key=construction:line:A_line_01;")
                && !missing.contains("selected_row=r0[key=construction:line:B_line_03;"),
            "missing selected row must not highlight another queue row, got: {missing}"
        );

        view.rebind(&rebind_ctx(&a));
        let restored = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, restored,
            "returning to A must restore the selected construction row exactly"
        );
    }

    #[test]
    fn construction_unkeyed_and_outside_row_clicks_are_inert() {
        let snap = snap_construction_missing_first_identity();
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));

        let unkeyed = click_construction_row(&mut view, 0);
        assert!(
            !unkeyed.consumed && !unkeyed.needs_redraw && unkeyed.commands.is_empty(),
            "row without measured construction identity must not be selected by index"
        );
        assert!(view.selected_queue_key.is_none());

        let viewport = construction_grid_fallback(view.screen);
        let outside = view.on_input(&ViewInput::PointerDown {
            x: viewport.x + viewport.w + 20.0,
            y: viewport.y + 4.0,
            button: PointerButton::Left,
        });
        assert!(
            !outside.consumed && !outside.needs_redraw && outside.commands.is_empty(),
            "outside construction-row click must be inert"
        );

        let keyed = click_construction_row(&mut view, 1);
        assert!(keyed.consumed && keyed.needs_redraw);
        assert_eq!(
            view.selected_queue_key.as_deref(),
            Some("construction:line:keyed_line")
        );
    }

    #[test]
    fn construction_queue_wheel_scroll_changes_visible_rows_and_clamps() {
        let stage = stage_with(&["construction.json"]);
        let snap = many_construction_snap(40, "A");
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));

        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(construction_scroll_pair(&before).0, 0);
        assert!(
            visible_construction_chunk(&before)
                .contains("key=construction:line:A_line_01;picture=1(D);"),
            "{before}"
        );

        let out = wheel_construction(&mut view, -3.0);
        assert!(out.consumed && out.needs_redraw);
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(construction_scroll_pair(&after).0, 156);
        assert!(
            visible_construction_chunk(&after)
                .contains("key=construction:line:A_line_04;picture=4(D);"),
            "after three-row wheel scroll, visible rows must advance: {after}"
        );
        assert!(
            !visible_construction_chunk(&after).contains("key=construction:line:A_line_01;"),
            "scrolled visible slice must not keep top rows: {after}"
        );

        let out = wheel_construction(&mut view, -500.0);
        assert!(out.consumed);
        let clamped = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let (cur, max) = construction_scroll_pair(&clamped);
        assert_eq!(cur, max, "huge wheel-down must clamp to max: {clamped}");

        let out = wheel_construction(&mut view, 500.0);
        assert!(out.consumed);
        let reset = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(construction_scroll_pair(&reset).0, 0);
    }

    #[test]
    fn construction_scrolled_row_click_selects_correct_backing_key() {
        let stage = stage_with(&[]);
        let snap = many_construction_snap(40, "A");
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));

        wheel_construction(&mut view, -5.0);
        let out = click_construction_row(&mut view, 0);
        assert!(out.consumed && out.needs_redraw);
        let d = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("selected_key=construction:line:A_line_06"),
            "after five-row scroll, visible row 0 must select backing construction row 6, got:\n{d}"
        );
        assert!(
            d.contains("selected_row=r0[key=construction:line:A_line_06;"),
            "selected digest must point at backing construction row 6, got:\n{d}"
        );
    }

    #[test]
    fn construction_scroll_survives_scrub_a_b_a_without_b_rows() {
        let stage = stage_with(&[]);
        let a = many_construction_snap(40, "A");
        let b = many_construction_snap(45, "B");
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&a));

        wheel_construction(&mut view, -5.0);
        click_construction_row(&mut view, 0);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert!(a1.contains("selected_key=construction:line:A_line_06"));
        assert!(
            visible_construction_chunk(&a1).contains("picture=6(D);"),
            "{a1}"
        );

        view.rebind(&rebind_ctx(&b));
        let b_digest = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            b_digest.contains("selected=missing_selected"),
            "B uses different measured construction ids, so A's selection must fail closed: {b_digest}"
        );
        assert!(
            visible_construction_chunk(&b_digest)
                .contains("key=construction:line:B_line_06;picture=6(D);"),
            "visible chunk must rebuild from B rows instead of carrying A text, got:\n{b_digest}"
        );
        assert!(
            !visible_construction_chunk(&b_digest).contains("key=construction:line:A_line_06;"),
            "B digest must not carry A-only row text, got:\n{b_digest}"
        );

        view.rebind(&rebind_ctx(&a));
        let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "construction scroll + selection must be byte-identical across A->B->A"
        );
    }

    #[test]
    fn construction_wheel_outside_grid_is_inert() {
        let snap = many_construction_snap(40, "A");
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));
        let viewport = construction_grid_fallback(view.screen);

        let out = view.on_input(&ViewInput::Wheel {
            x: viewport.x + viewport.w + 20.0,
            y: viewport.y + CONSTRUCTION_ROW_H,
            dy: -5.0,
        });
        assert!(
            !out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "wheel outside the construction queue grid must be inert"
        );
        assert_eq!(view.queue_scroll_y, 0.0);
    }

    #[test]
    fn construction_scrollbar_drag_changes_visible_rows_and_clamps() {
        let stage = stage_with(&[]);
        let snap = many_construction_snap(40, "A");
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(construction_scroll_pair(&before).0, 0);

        let down = pointer_down_construction_thumb(&mut view);
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "thumb pointer-down should consume to capture construction drag without changing scroll"
        );
        let geom = construction_scrollbar(&view);
        let drag = view.on_input(&ViewInput::PointerMove {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.track.y + geom.track.h + 500.0,
            drag: true,
        });
        assert!(drag.consumed && drag.needs_redraw);
        let bottom = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let (cur, max) = construction_scroll_pair(&bottom);
        assert_eq!(cur, max, "dragging below the track should clamp to max");
        assert!(
            !visible_construction_chunk(&bottom).contains("key=construction:line:A_line_01;"),
            "bottom-dragged visible construction chunk must not keep top rows: {bottom}"
        );

        let reset = view.on_input(&ViewInput::PointerMove {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.track.y - 500.0,
            drag: true,
        });
        assert!(reset.consumed && reset.needs_redraw);
        let top = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(construction_scroll_pair(&top).0, 0);
        assert!(
            visible_construction_chunk(&top).contains("key=construction:line:A_line_01;"),
            "dragging above the track should clamp back to top rows: {top}"
        );
    }

    #[test]
    fn construction_scrollbar_drag_capture_continues_outside_and_clears_on_up() {
        let snap = many_construction_snap(40, "A");
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));
        let geom = construction_scrollbar(&view);

        let down = pointer_down_construction_thumb(&mut view);
        assert!(down.consumed && !down.needs_redraw);

        let drag = view.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 300.0,
            y: geom.track.y + geom.track.h,
            drag: true,
        });
        assert!(
            drag.consumed && drag.needs_redraw,
            "captured drag should continue even when x leaves the construction grid"
        );
        assert!(view.queue_scroll_y > 0.0);

        let up = view.on_input(&ViewInput::PointerUp {
            x: geom.track.x - 300.0,
            y: geom.track.y + geom.track.h,
            button: PointerButton::Left,
        });
        assert!(
            up.consumed && !up.needs_redraw,
            "pointer-up should consume solely to clear construction scrollbar capture"
        );
        let ignored = view.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 300.0,
            y: geom.track.y,
            drag: true,
        });
        assert!(
            !ignored.consumed && !ignored.needs_redraw,
            "pointer move after thumb release must not keep stale construction scrollbar capture"
        );
    }

    #[test]
    fn construction_scrollbar_track_click_does_not_select_row() {
        let stage = stage_with(&[]);
        let snap = many_construction_snap(40, "A");
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let geom = construction_scrollbar(&view);
        let track_y = (geom.thumb.y + geom.thumb.h + 10.0).min(geom.track.y + geom.track.h - 1.0);
        let out = view.on_input(&ViewInput::PointerDown {
            x: geom.track.x + geom.track.w * 0.5,
            y: track_y,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "clicking the construction scrollbar track outside the thumb should be an inert consumed UI hit"
        );
        assert!(
            view.selected_queue_key.is_none(),
            "construction scrollbar track click must not select an underlying queue row"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "track click must not alter selected construction digest, got: {after}"
        );
    }

    #[test]
    fn construction_count_container_factory_buttons_are_replay_disabled_noops() {
        let stage = stage_with(&[]);
        let snap = many_construction_snap(40, "A");
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));

        wheel_construction(&mut view, -4.0);
        let select = click_construction_row(&mut view, 0);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected_key=construction:line:A_line_05"));
        let before_scroll = construction_scroll_pair(&before);
        let before_selected = view.selected_queue_key.clone();

        let construction_gui = std::fs::read_to_string(gui_dir().join(CONSTRUCTION_FILE))
            .expect("real construction GUI is available for count-button anchor checks");
        for name in CONSTRUCTION_FACTORY_COUNT_BUTTON_NAMES {
            assert!(
                construction_gui.contains(&format!(r#""name": "{name}""#)),
                "real construction GUI must keep the {name} count-container button"
            );
        }

        // The 1x/5x/10x variants share two physical positions in construction.json:
        // subtract variants overlap, add variants overlap. The replay consumes those
        // two visible hit areas as disabled no-op input.
        for (x, y) in construction_factory_count_centers(&view) {
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                down.consumed && !down.needs_redraw && down.commands.is_empty(),
                "factory add/sub pointer-down must be a replay-disabled no-op"
            );
            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                up.consumed && !up.needs_redraw && up.commands.is_empty(),
                "factory add/sub pointer-up must be a replay-disabled no-op"
            );
        }

        assert_eq!(
            view.selected_queue_key, before_selected,
            "disabled factory count buttons must not change selected construction row"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(construction_scroll_pair(&after), before_scroll);
        assert_eq!(
            before, after,
            "disabled factory count buttons must not change scroll, selection, rows, or digest"
        );
    }

    #[test]
    fn construction_queue_buttons_are_replay_disabled_noops() {
        let Some(gui) = load_gui(CONSTRUCTION_FILE) else {
            eprintln!("[construction] skipped queue-button test: game_assets/gui not extracted");
            return;
        };
        let template = find_raw_window(&gui.windows, "production_building_line_entry")
            .expect("production_building_line_entry template exists");
        for name in CONSTRUCTION_QUEUE_BUTTON_NAMES {
            assert!(
                find_raw_child(template, name).is_some(),
                "real construction row template must expose {name}"
            );
        }

        let stage = stage_with(&[]);
        let snap = many_construction_snap(40, "A");
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));

        wheel_construction(&mut view, -4.0);
        let select = click_construction_row(&mut view, 0);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected_key=construction:line:A_line_05"));
        let before_scroll = construction_scroll_pair(&before);
        let before_selected = view.selected_queue_key.clone();

        for (x, y) in construction_queue_button_centers(&view, 0) {
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                down.consumed && !down.needs_redraw && down.commands.is_empty(),
                "construction queue button pointer-down must be a replay-disabled no-op"
            );
            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                up.consumed && !up.needs_redraw && up.commands.is_empty(),
                "construction queue button pointer-up must be a replay-disabled no-op"
            );
        }

        assert_eq!(
            view.selected_queue_key, before_selected,
            "disabled queue buttons must not change selected construction row"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(construction_scroll_pair(&after), before_scroll);
        assert_eq!(
            before, after,
            "disabled queue buttons must not change scroll, selection, rows, or digest"
        );
    }

    #[test]
    fn construction_temp_button_is_replay_disabled_noop() {
        let Some(gui) = load_gui(CONSTRUCTION_FILE) else {
            eprintln!("[construction] skipped temp-button test: game_assets/gui not extracted");
            return;
        };
        let node = find_raw_node(&gui.windows, CONSTRUCTION_TEMP_BUTTON_NAME)
            .expect("construction temp/backhoe button exists in root layout");
        assert_eq!(node.kind, "buttonType");
        assert_eq!(node.sprite.as_deref(), Some("GFX_backhoe_icon_small"));

        let stage = stage_with(&[]);
        let snap = many_construction_snap(40, "A");
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));

        wheel_construction(&mut view, -4.0);
        let select = click_construction_row(&mut view, 0);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected_key=construction:line:A_line_05"));
        let before_scroll = construction_scroll_pair(&before);
        let before_selected = view.selected_queue_key.clone();
        let (x, y) = construction_temp_button_center(&view);

        let down = view.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "construction temp/backhoe pointer-down must be a replay-disabled no-op"
        );
        let up = view.on_input(&ViewInput::PointerUp {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            up.consumed && !up.needs_redraw && up.commands.is_empty(),
            "construction temp/backhoe pointer-up must be a replay-disabled no-op"
        );

        assert_eq!(
            view.selected_queue_key, before_selected,
            "disabled temp/backhoe button must not change selected construction row"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(construction_scroll_pair(&after), before_scroll);
        assert_eq!(
            before, after,
            "disabled temp/backhoe button must not change scroll, selection, rows, or digest"
        );
    }

    #[test]
    fn construction_queue_buttons_do_not_claim_neighboring_row_clicks() {
        let stage = stage_with(&[]);
        let snap = many_construction_snap(40, "A");

        for (label, rect_idx, x_offset) in [
            ("priority", 0usize, -16.0f32),
            ("amount", 2usize, CONSTRUCTION_QUEUE_BUTTON_SIZE + 10.0),
            ("close", 4usize, -4.0f32),
        ] {
            let mut view = ConstructionView::new();
            view.rebind(&rebind_ctx(&snap));
            wheel_construction(&mut view, -4.0);
            let viewport = construction_grid_fallback(view.screen);
            let rect = construction_queue_button_rects(viewport, viewport.y)[rect_idx];
            let x = rect.x + x_offset;
            let y = rect.y + rect.h * 0.5;
            let out = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                out.consumed && out.needs_redraw && out.commands.is_empty(),
                "clicks beside construction {label} buttons must still select the visible backing row"
            );
            let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
            assert!(
                digest.contains("selected_key=construction:line:A_line_05"),
                "neighboring construction {label} row click should select the scrolled row, got: {digest}"
            );
        }
    }

    #[test]
    fn construction_count_container_non_button_clicks_are_unclaimed() {
        let stage = stage_with(&[]);
        let snap = many_construction_snap(40, "A");
        let mut view = ConstructionView::new();
        view.rebind(&rebind_ctx(&snap));
        wheel_construction(&mut view, -2.0);
        click_construction_row(&mut view, 0);

        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let rects = construction_factory_count_button_rects(view.screen);
        let x = rects[0].x + rects[0].w + (rects[1].x - rects[0].x - rects[0].w) * 0.5;
        let y = rects[0].y + rects[0].h * 0.5;
        let out = view.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });

        assert!(
            !out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "clicks in the count container gap must stay unclaimed instead of becoming broad modal blocking"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "non-button count-container clicks must not mutate construction view state"
        );
    }

    #[test]
    fn decisions_scrub_a_b_a_byte_identical() {
        let stage = stage_with(&["decisions.json"]);
        let (a, b) = (snap_save(), snap_live());
        let v = DecisionsView::new();
        let da1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        let db = bound_digest(&v.bind(&ctx(&stage, &b)));
        assert_ne!(da1, db, "save (empty timed) and live (1 timed) must differ");
        let da2 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert_eq!(
            da1, da2,
            "scrub A→B→A: decisions carried a value from the B visit"
        );
    }

    // -------- no-fabrication: a blocked field renders "—", never a 0 ----------------------

    #[test]
    fn blocked_fields_render_dash_not_zero() {
        // SAVE snapshot: live production line PRIORITY is absent, efficiency bars absent,
        // construction location is the live-resisted case → all must be "—", never "0".
        let stage = stage_with(&["production.json"]);
        let snap = snap_save();
        let v = ProductionView::new();
        let bv = v.bind(&ctx(&stage, &snap));
        let d = bound_digest(&bv);
        // the unavailable summary aggregates (efficiency cap/output) render the glyph.
        assert!(
            d.contains('—'),
            "an unavailable field must render the not-captured glyph"
        );
        // a measured line field (active_factories=25) renders its real value, not "—".
        assert!(
            d.contains("factories_used_txt=25(E)"),
            "measured active factories must show 25"
        );
        // the per-line efficiency bars (not in the save shape) are badged unavailable.
        assert!(
            d.contains("efficiency_progressbar=—(U)"),
            "per-line efficiency must badge unavailable"
        );
    }

    #[test]
    fn decision_status_badges_unavailable_not_fabricated() {
        // the live timed-decision row binds name+days, but decision_status (an OPEN
        // extraction gap) must badge unavailable "—", never a fabricated 0/false.
        let stage = stage_with(&["decisions.json"]);
        let snap = snap_live();
        let v = DecisionsView::new();
        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("name_text=war_economy_decision(D)"),
            "decision id token bound (derived)"
        );
        assert!(d.contains("timer_text=42(E)"), "days-remaining bound exact");
        assert!(
            d.contains("decision_status=—(gap)"),
            "decision_status must badge an unavailable gap"
        );
    }

    #[test]
    fn decision_status_shows_measured_present_set_when_recorded() {
        // RE-WIRE: with the live `decisions.status_present` flag now recorded, the
        // decision_status gap reflects the MEASURED present-status set (no fabrication; the
        // value comes straight from the snapshot, "—" only when truly absent).
        let stage = stage_with(&["decisions.json"]);
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "economy": {}, "production": {},
                "decisions": {
                    "timed_live": [
                        {"decision": "war_economy_decision", "days": 42, "state": 64}
                    ],
                    "status_present": { "active_timed_decision": 1 }
                }
            } }
        }))
        .unwrap();
        let v = DecisionsView::new();
        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("decision_status=active_timed_decision(gap)"),
            "decision_status must surface the measured present-status set, got: {d}"
        );
        assert!(
            !d.contains("decision_status=—(gap)"),
            "a measured status must not badge —"
        );
    }

    #[test]
    fn decision_status_empty_present_set_is_none_not_fabricated() {
        // a MEASURED-EMPTY present set ({}) renders "none" (a real read of "no statuses"),
        // distinct from an ABSENT flag ("—"). Neither is a fabricated 0/false.
        let stage = stage_with(&["decisions.json"]);
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "countries": { "GER": {
                "economy": {}, "production": {},
                "decisions": {
                    "timed_live": [ {"decision": "war_economy_decision", "days": 9} ],
                    "status_present": {}
                }
            } }
        }))
        .unwrap();
        let v = DecisionsView::new();
        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("decision_status=none(gap)"),
            "measured-empty -> none, got: {d}"
        );
    }

    #[test]
    fn decisions_timed_row_click_selects_measured_decision_state_key() {
        let stage = stage_with(&[]);
        let snap = snap_decisions_a();
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&snap));

        let default = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            default.contains("selected=none"),
            "pre-click decisions view should not claim a selection, got: {default}"
        );

        let out = click_decision_row(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        assert!(out.commands.is_empty());

        let selected = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected=selected"),
            "row click must select a timed decision row, got: {selected}"
        );
        assert!(
            selected.contains("selected_key=decision:repeat_decision:state:75"),
            "selection must use measured decision+state identity, got: {selected}"
        );
        assert!(
            selected.contains("selected_row=r0[key=decision:repeat_decision:state:75;"),
            "selected row digest must point at the clicked backing row, got: {selected}"
        );
        assert!(
            selected.contains("timer_text=8(E)"),
            "clicked second row's timer must be visible in selected digest, got: {selected}"
        );
    }

    #[test]
    fn decisions_timed_select_button_is_replay_disabled_noop() {
        let stage = stage_with(&["decisions.json"]);
        let snap = snap_decisions_a();
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&snap));

        let select = click_decision_row(&mut view, 1);
        assert!(select.consumed && select.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected_key=decision:repeat_decision:state:75"));
        let before_scroll = decisions_scroll_pair(&before);
        let before_selected = view.selected_timed_key.clone();
        let (x, y) = decision_select_button_center(&view, 0);

        let down = view.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "timed decision btn_select pointer-down must be a replay-disabled no-op"
        );
        let up = view.on_input(&ViewInput::PointerUp {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            up.consumed && !up.needs_redraw && up.commands.is_empty(),
            "timed decision btn_select pointer-up must be a replay-disabled no-op"
        );

        assert_eq!(
            view.selected_timed_key, before_selected,
            "disabled btn_select must not change selected timed decision row"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(decisions_scroll_pair(&after), before_scroll);
        assert_eq!(
            before, after,
            "disabled btn_select must not change scroll, selection, rows, or digest"
        );
    }

    #[test]
    fn decisions_select_button_does_not_claim_neighboring_row_body_clicks() {
        let stage = stage_with(&[]);
        let snap = snap_decisions_a();
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&snap));
        let viewport = decisions_grid_fallback(view.screen);
        let rect = decision_select_button_rect(viewport, viewport.y + DECISION_ROW_H);

        let out = view.on_input(&ViewInput::PointerDown {
            x: rect.x - 4.0,
            y: rect.y + rect.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && out.needs_redraw && out.commands.is_empty(),
            "clicks beside timed-decision btn_select must still select the visible backing row"
        );
        let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            digest.contains("selected_key=decision:repeat_decision:state:75"),
            "neighboring timed-decision row body click should select row 1, got: {digest}"
        );
    }

    #[test]
    fn decisions_scrolled_select_button_is_replay_disabled_noop_for_visible_row() {
        let stage = stage_with(&["decisions.json"]);
        let snap = many_decisions_snap(40, "A");
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&snap));

        let scroll = wheel_decisions(&mut view, -5.0);
        assert!(scroll.consumed && scroll.needs_redraw);
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(decisions_scroll_pair(&before).0, 200);
        assert!(
            visible_decisions_chunk(&before)
                .contains("key=decision:A_decision_06:state:6;name_text=A_decision_06(D);"),
            "scrolled visible row 0 must be backed by decision 6 before the button click: {before}"
        );
        let before_selected = view.selected_timed_key.clone();
        let (x, y) = decision_select_button_center(&view, 0);

        let down = view.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "scrolled timed-decision btn_select pointer-down must be a replay-disabled no-op"
        );
        let up = view.on_input(&ViewInput::PointerUp {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            up.consumed && !up.needs_redraw && up.commands.is_empty(),
            "scrolled timed-decision btn_select pointer-up must be a replay-disabled no-op"
        );

        assert_eq!(
            view.selected_timed_key, before_selected,
            "scrolled btn_select must not select its visible timed-decision row"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "scrolled btn_select must not change scroll, selection, rows, or digest"
        );
        assert!(
            after.contains("selected=none"),
            "scrolled btn_select must leave the decisions view unselected, got: {after}"
        );
    }

    #[test]
    fn decisions_selected_row_scrub_a_b_a_rebuilds_current_hour_row() {
        let stage = stage_with(&[]);
        let a = snap_decisions_a();
        let b = snap_decisions_b_same_selected();
        let mut view = DecisionsView::new();

        view.rebind(&rebind_ctx(&a));
        let out = click_decision_row(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert!(a1.contains("selected_key=decision:repeat_decision:state:75"));
        assert!(a1.contains("timer_text=8(E)"));

        view.rebind(&rebind_ctx(&b));
        let b_digest = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            b_digest.contains("selected_key=decision:repeat_decision:state:75"),
            "selected key must survive into B, got: {b_digest}"
        );
        assert!(
            b_digest.contains("selected_row=r0[key=decision:repeat_decision:state:75;"),
            "B selected-row digest must still point at the selected backing row, got: {b_digest}"
        );
        assert!(
            b_digest.contains("timer_text=99(E)"),
            "B selected row must rebuild from B's current row, got: {b_digest}"
        );

        view.rebind(&rebind_ctx(&a));
        let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "selected decision scrub A->B->A must be byte-identical"
        );
    }

    #[test]
    fn decisions_missing_selected_row_stays_selected_missing_without_row_fallback() {
        let stage = stage_with(&[]);
        let a = snap_decisions_a();
        let b = snap_decisions_b_missing_selected();
        let mut view = DecisionsView::new();

        view.rebind(&rebind_ctx(&a));
        let out = click_decision_row(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));

        view.rebind(&rebind_ctx(&b));
        let missing = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            missing.contains("selected=missing_selected"),
            "missing selected decision must be explicit, got: {missing}"
        );
        assert!(
            missing.contains("selected_key=decision:repeat_decision:state:75"),
            "missing digest must retain the requested key, got: {missing}"
        );
        assert!(
            missing.contains("selected_row=none"),
            "missing selected row must not fall back to another row, got: {missing}"
        );
        assert!(
            !missing.contains("selected_row=r0[key=decision:repeat_decision:state:64;"),
            "missing selected row must not highlight the remaining first row, got: {missing}"
        );

        view.rebind(&rebind_ctx(&a));
        let restored = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, restored,
            "returning to A must restore the selected row exactly"
        );
    }

    #[test]
    fn decisions_unkeyed_and_outside_row_clicks_are_inert() {
        let snap = snap_decisions_missing_first_identity();
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&snap));

        let unkeyed = click_decision_row(&mut view, 0);
        assert!(
            !unkeyed.consumed && !unkeyed.needs_redraw && unkeyed.commands.is_empty(),
            "row without measured decision id must not be selected by index"
        );
        assert!(view.selected_timed_key.is_none());

        let viewport = decisions_grid_fallback(view.screen);
        let outside = view.on_input(&ViewInput::PointerDown {
            x: viewport.x + viewport.w + 20.0,
            y: viewport.y + 4.0,
            button: PointerButton::Left,
        });
        assert!(
            !outside.consumed && !outside.needs_redraw && outside.commands.is_empty(),
            "outside-row click must be inert"
        );

        let keyed = click_decision_row(&mut view, 1);
        assert!(keyed.consumed && keyed.needs_redraw);
        assert_eq!(
            view.selected_timed_key.as_deref(),
            Some("decision:repeat_decision:state:75")
        );
    }

    #[test]
    fn decisions_timed_wheel_scroll_changes_visible_rows_and_clamps() {
        let stage = stage_with(&["decisions.json"]);
        let snap = many_decisions_snap(40, "A");
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&snap));

        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(decisions_scroll_pair(&before).0, 0);
        assert!(
            visible_decisions_chunk(&before)
                .contains("key=decision:A_decision_01:state:1;name_text=A_decision_01(D);"),
            "{before}"
        );

        let out = wheel_decisions(&mut view, -3.0);
        assert!(out.consumed && out.needs_redraw);
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(decisions_scroll_pair(&after).0, 120);
        assert!(
            visible_decisions_chunk(&after)
                .contains("key=decision:A_decision_04:state:4;name_text=A_decision_04(D);"),
            "after three-row wheel scroll, visible rows must advance: {after}"
        );
        assert!(
            !visible_decisions_chunk(&after).contains("key=decision:A_decision_01:state:1;"),
            "scrolled visible slice must not keep top rows: {after}"
        );

        let out = wheel_decisions(&mut view, -500.0);
        assert!(out.consumed);
        let clamped = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let (cur, max) = decisions_scroll_pair(&clamped);
        assert_eq!(cur, max, "huge wheel-down must clamp to max: {clamped}");

        let out = wheel_decisions(&mut view, 500.0);
        assert!(out.consumed);
        let reset = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(decisions_scroll_pair(&reset).0, 0);
    }

    #[test]
    fn decisions_scrolled_row_click_selects_correct_backing_key() {
        let stage = stage_with(&[]);
        let snap = many_decisions_snap(40, "A");
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&snap));

        wheel_decisions(&mut view, -5.0);
        let out = click_decision_row(&mut view, 0);
        assert!(out.consumed && out.needs_redraw);
        let d = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("selected_key=decision:A_decision_06:state:6"),
            "after five-row scroll, visible row 0 must select backing decision 6, got:\n{d}"
        );
        assert!(
            d.contains("selected_row=r0[key=decision:A_decision_06:state:6;"),
            "selected digest must point at backing decision 6, got:\n{d}"
        );
    }

    #[test]
    fn decisions_scroll_survives_scrub_a_b_a_without_b_rows() {
        let stage = stage_with(&[]);
        let a = many_decisions_snap(40, "A");
        let b = many_decisions_snap(45, "B");
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&a));

        wheel_decisions(&mut view, -5.0);
        click_decision_row(&mut view, 0);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert!(a1.contains("selected_key=decision:A_decision_06:state:6"));
        assert!(
            visible_decisions_chunk(&a1).contains("name_text=A_decision_06(D);"),
            "{a1}"
        );

        view.rebind(&rebind_ctx(&b));
        let b_digest = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            b_digest.contains("selected=missing_selected"),
            "B uses different measured decision ids, so A's selection must fail closed: {b_digest}"
        );
        assert!(
            visible_decisions_chunk(&b_digest).contains("name_text=B_decision_06(D);"),
            "visible chunk must rebuild from B rows instead of carrying A text, got:\n{b_digest}"
        );
        assert!(
            !visible_decisions_chunk(&b_digest).contains("name_text=A_decision_06(D);"),
            "B digest must not carry A-only row text, got:\n{b_digest}"
        );

        view.rebind(&rebind_ctx(&a));
        let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "decision scroll + selection must be byte-identical across A->B->A"
        );
    }

    #[test]
    fn decisions_wheel_outside_grid_is_inert() {
        let snap = many_decisions_snap(40, "A");
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&snap));
        let viewport = decisions_grid_fallback(view.screen);

        let out = view.on_input(&ViewInput::Wheel {
            x: viewport.x + viewport.w + 20.0,
            y: viewport.y + DECISION_ROW_H,
            dy: -5.0,
        });
        assert!(
            !out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "wheel outside the timed-decision grid must be inert"
        );
        assert_eq!(view.timed_scroll_y, 0.0);
    }

    #[test]
    fn decisions_scrollbar_drag_changes_visible_rows_and_clamps() {
        let stage = stage_with(&["decisions.json"]);
        let snap = many_decisions_snap(40, "A");
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&snap));
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(decisions_scroll_pair(&before).0, 0);

        let down = pointer_down_decision_thumb(&mut view);
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "thumb pointer-down should consume to capture decision drag without changing scroll"
        );
        let geom = decision_scrollbar(&view);
        let drag = view.on_input(&ViewInput::PointerMove {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.track.y + geom.track.h + 500.0,
            drag: true,
        });
        assert!(drag.consumed && drag.needs_redraw);
        let bottom = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let (cur, max) = decisions_scroll_pair(&bottom);
        assert_eq!(cur, max, "dragging below the track should clamp to max");
        assert!(
            !visible_decisions_chunk(&bottom).contains("key=decision:A_decision_01:state:1;"),
            "bottom-dragged visible decision chunk must not keep top rows: {bottom}"
        );

        let reset = view.on_input(&ViewInput::PointerMove {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.track.y - 500.0,
            drag: true,
        });
        assert!(reset.consumed && reset.needs_redraw);
        let top = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(decisions_scroll_pair(&top).0, 0);
        assert!(
            visible_decisions_chunk(&top).contains("key=decision:A_decision_01:state:1;"),
            "dragging above the track should clamp back to top rows: {top}"
        );
    }

    #[test]
    fn decisions_scrollbar_drag_capture_continues_outside_and_clears_on_up() {
        let snap = many_decisions_snap(40, "A");
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&snap));
        let geom = decision_scrollbar(&view);

        let down = pointer_down_decision_thumb(&mut view);
        assert!(down.consumed && !down.needs_redraw);

        let drag = view.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 300.0,
            y: geom.track.y + geom.track.h,
            drag: true,
        });
        assert!(
            drag.consumed && drag.needs_redraw,
            "captured drag should continue even when x leaves the decision grid"
        );
        assert!(view.timed_scroll_y > 0.0);

        let up = view.on_input(&ViewInput::PointerUp {
            x: geom.track.x - 300.0,
            y: geom.track.y + geom.track.h,
            button: PointerButton::Left,
        });
        assert!(
            up.consumed && !up.needs_redraw,
            "pointer-up should consume solely to clear decision scrollbar capture"
        );
        let ignored = view.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 300.0,
            y: geom.track.y,
            drag: true,
        });
        assert!(
            !ignored.consumed && !ignored.needs_redraw,
            "pointer move after thumb release must not keep stale decision scrollbar capture"
        );
    }

    #[test]
    fn decisions_scrollbar_track_click_does_not_select_row() {
        let stage = stage_with(&[]);
        let snap = many_decisions_snap(40, "A");
        let mut view = DecisionsView::new();
        view.rebind(&rebind_ctx(&snap));
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let geom = decision_scrollbar(&view);
        let track_y = (geom.thumb.y + geom.thumb.h + 10.0).min(geom.track.y + geom.track.h - 1.0);
        let out = view.on_input(&ViewInput::PointerDown {
            x: geom.track.x + geom.track.w * 0.5,
            y: track_y,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "clicking the decision scrollbar track outside the thumb should be an inert consumed UI hit"
        );
        assert!(
            view.selected_timed_key.is_none(),
            "decision scrollbar track click must not select an underlying timed-decision row"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "track click must not alter selected decision digest, got: {after}"
        );
    }

    #[test]
    fn absent_section_is_not_conflated_with_empty() {
        // SAVE snapshot has NO decisions.timed_live AND NO decisions.timed → the section is
        // ABSENT (the row factory emits 0 rows) and the digest flags it 'absent', NOT a
        // fabricated 'present empty list'.
        let stage = stage_with(&["decisions.json"]);
        // a snapshot whose GER decisions object has neither timed_live nor timed.
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12",
            "countries": { "GER": { "economy": {}, "decisions": { "active": [] } } }
        }))
        .unwrap();
        let v = DecisionsView::new();
        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("sect=absent"),
            "absent timed-decision section must badge absent, got: {d}"
        );
        // and a MEASURED empty list (save snap has timed:[]) is 'present', not 'absent'.
        let d2 = bound_digest(&v.bind(&ctx(&stage, &snap_save())));
        assert!(
            d2.contains("sect=present"),
            "a measured empty timed list must badge present"
        );
    }

    // -------- per-view element accounting: scene walk == the .gui's element_type_counts ---

    /// Count `type` tokens recursively over the RAW gui json (the authoritative element set
    /// — incl. layout primitives borderSize/slotsize/etc, which the typed scene drops).
    fn raw_type_counts(name: &str) -> HashMap<String, usize> {
        let p = gui_dir().join(name);
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let mut counts: HashMap<String, usize> = HashMap::new();
        fn walk(node: &serde_json::Value, counts: &mut HashMap<String, usize>) {
            if let Some(t) = node.get("type").and_then(|t| t.as_str()) {
                *counts.entry(t.to_string()).or_insert(0) += 1;
            }
            if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
                for c in children {
                    walk(c, counts);
                }
            }
        }
        if let Some(windows) = v.get("windows").and_then(|w| w.as_array()) {
            for w in windows {
                walk(w, &mut counts);
            }
        }
        counts
    }

    /// The file's OWN declared element_type_counts (under `stats`).
    fn declared_counts(name: &str) -> HashMap<String, usize> {
        let p = gui_dir().join(name);
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let etc = v
            .get("stats")
            .and_then(|s| s.get("element_type_counts"))
            .and_then(|e| e.as_object())
            .expect("element_type_counts present");
        etc.iter()
            .map(|(k, v)| (k.clone(), v.as_u64().unwrap() as usize))
            .collect()
    }

    /// For each of the 3 views' `.gui` files, the element set we account for (the recursive
    /// `type`-token walk over the parsed tree) EQUALS the file's declared element_type_counts
    /// — proving every element of each view is accounted (none silently dropped).
    #[test]
    fn element_accounting_production() {
        assert_eq!(
            raw_type_counts("production.json"),
            declared_counts("production.json")
        );
    }
    #[test]
    fn element_accounting_construction() {
        assert_eq!(
            raw_type_counts("construction.json"),
            declared_counts("construction.json")
        );
    }
    #[test]
    fn element_accounting_decisions() {
        assert_eq!(
            raw_type_counts("decisions.json"),
            declared_counts("decisions.json")
        );
    }

    /// The accounting totals match the binding-map's stated parsed-node counts (663/149/103),
    /// a guard that the source files haven't drifted under the map.
    #[test]
    fn element_accounting_totals_match_binding_map() {
        let total = |n: &str| raw_type_counts(n).values().sum::<usize>();
        assert_eq!(total("production.json"), 663);
        assert_eq!(total("construction.json"), 149);
        assert_eq!(total("decisions.json"), 103);
    }

    /// Collect every `name` in a `.gui` window subtree (the named element set), walking the
    /// RAW json (lenient — independent of the typed `GuiFile` parser, like the accounting test).
    fn window_names(name: &str, root: &str) -> std::collections::HashSet<String> {
        let p = gui_dir().join(name);
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let mut out = std::collections::HashSet::new();
        fn walk(node: &serde_json::Value, out: &mut std::collections::HashSet<String>) {
            if let Some(nm) = node.get("name").and_then(|n| n.as_str()) {
                out.insert(nm.to_string());
            }
            if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
                for c in children {
                    walk(c, out);
                }
            }
        }
        if let Some(windows) = v.get("windows").and_then(|w| w.as_array()) {
            if let Some(w) = windows
                .iter()
                .find(|w| w.get("name").and_then(|n| n.as_str()) == Some(root))
            {
                walk(w, &mut out);
            }
        }
        out
    }

    /// BINDING-COVERAGE guard (Codex point 5): every element name I bind a `Cell` to MUST
    /// exist as a real node in its target window (the ROOT for summary cells, the row
    /// TEMPLATE for row cells) — else `set_element_text` silently no-ops and the field is
    /// neither shown nor honestly "—". This catches a typo'd element name the element_type
    /// accounting cannot. The (element, window-root) pairs below mirror the `bind` code.
    #[test]
    fn every_bound_element_name_exists_in_its_window() {
        // (file, root-or-template, [element names bound there])
        let cases: &[(&str, &str, &[&str])] = &[
            // production summary -> the ROOT window.
            (
                "production.json",
                "countryproductionlineview",
                &[
                    "operational_factories_value",
                    "operational_naval_factories_value",
                    "operational_naval_factories_repair_value",
                    "docyards_output_value",
                    "production_output_value",
                    "efficiency_cap_value",
                    "efficiency_growth_value",
                    "production_efficiency_loss_value",
                    "factory_bombing_defence_value",
                ],
            ),
            // production line rows -> the entry TEMPLATE. `amount` exists ONLY on naval/
            // railway templates, so the military case omits it (matching `bind_line_row`).
            (
                "production.json",
                "production_military_line_entry",
                &[
                    "name",
                    "general_name",
                    "factories_used_txt",
                    "speed",
                    "priority",
                    "progressbar",
                    "total_need",
                    "efficiency_progressbar",
                    "efficiency_progressbar_to_max",
                ],
            ),
            (
                "production.json",
                "production_naval_line_entry",
                &[
                    "name",
                    "general_name",
                    "factories_used_txt",
                    "amount",
                    "speed",
                    "priority",
                    "progressbar",
                ],
            ),
            // the resource strip row.
            ("production.json", "resource_strip_item", &["value"]),
            // construction summary -> ROOT.
            (
                "construction.json",
                "countryconstructionsview",
                &[
                    "available_factories_value",
                    "local_factories_value",
                    "traded_factories_value",
                    "cic_bank_value",
                    "construction_speed_value",
                ],
            ),
            // construction queue rows -> the building-line TEMPLATE.
            (
                "construction.json",
                "production_building_line_entry",
                &[
                    "picture",
                    "factories",
                    "location",
                    "progress",
                    "queued",
                    "relative_flag",
                ],
            ),
            // decisions timed rows -> the timed-decision TEMPLATE.
            (
                "decisions.json",
                "timed_decision_item",
                &[
                    "name_text",
                    "timer_text",
                    "cost_and_timer_text",
                    "btn_progress_good",
                    "btn_progress_bad",
                ],
            ),
        ];
        let mut missing = Vec::new();
        for (file, root, elems) in cases {
            let names = window_names(file, root);
            for e in *elems {
                if !names.contains(*e) {
                    missing.push(format!("{file}>{root}: element `{e}` not found"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "bound element names with no matching .gui node: {missing:#?}"
        );
    }

    /// Every routed view is collected from the inventory under the production cluster.
    #[test]
    fn cluster_views_registered() {
        use crate::host::ViewRegistry;
        let reg = ViewRegistry::from_inventory();
        for id in [
            "production.production",
            "production.construction",
            "production.decisions",
        ] {
            assert!(
                reg.view(ViewId(id)).is_some(),
                "view {id} must be inventory-collected"
            );
        }
        let bar = reg.menubar();
        let group = bar
            .iter()
            .find(|g| g.cluster == MenuCluster::ProductionConstructionDecisions)
            .unwrap();
        assert_eq!(
            group.items.len(),
            3,
            "all 3 cluster views routed to the Production menu"
        );
    }
}
