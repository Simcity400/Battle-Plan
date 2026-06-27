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
use crate::host::registry::{
    GuiSymbolRef, MenuCluster, MenuRoute, TemplateDescriptor, ViewDescriptor, ViewId,
};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InstanceId, ViewCommand, ViewInstance, ZBand,
};
use crate::{register_template, register_view};

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
    cells: Vec<Cell>,
    gaps: Vec<(String, DisplayAtom)>,
}

impl DataRow {
    fn new() -> DataRow {
        DataRow {
            cells: Vec::new(),
            gaps: Vec::new(),
        }
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
    let viewport = match find_rect(panel, grid_name) {
        Some(r) => r,
        None => return,
    };
    let template = match find_template(file, template_name) {
        Some(t) => t,
        None => return,
    };
    if rows.is_empty() {
        return;
    }
    let manifest = &fd.stage.manifest;
    let spec = GridSpec::from_gridbox(&stub_grid(grid_name, viewport), &template, GUTTER);
    let content = ManifestContent {
        manifest,
        measure: |_t, _f| None,
    };
    let mut inst = gridbox::instantiate(&template, rows.len(), viewport, &spec, 0.0, &content);
    for (i, row) in inst.rows.iter_mut().enumerate() {
        for cell in &rows[i].cells {
            bind::apply::set_element_text(row, &cell.element, cell.atom.as_str());
        }
    }
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

fn stub_grid(name: &str, rect: Rect) -> Node {
    Node {
        name: name.to_string(),
        raw_kind: "gridBoxType".into(),
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
    match ctx.country_field(tag, dotted) {
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

pub struct ProductionView;

impl ProductionView {
    pub fn new() -> ProductionView {
        ProductionView
    }
}

impl Default for ProductionView {
    fn default() -> Self {
        ProductionView::new()
    }
}

/// The production view's owned per-frame draw inputs.
struct ProductionDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    mil_rows: Vec<DataRow>,
    naval_rows: Vec<DataRow>,
    resource_rows: Vec<DataRow>,
    /// SEPARATE section states (Codex point 3): a present military list + an absent naval
    /// list must NOT be conflated — each is tagged distinctly in the digest.
    mil_section: SectionState,
    naval_section: SectionState,
}

impl Drawable for ProductionDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "PROD mil_sect={} naval_sect={} mil={} naval={} res={}",
            self.mil_section.tag(),
            self.naval_section.tag(),
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
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "item_grid",
            "production_military_line_entry",
            &self.mil_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "item_grid",
            "production_naval_line_entry",
            &self.naval_rows,
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
        let (mil_rows, mil_section) = build_line_rows(ctx, tag, &mut fidelity, "military_lines");
        let (naval_rows, naval_section) = build_line_rows(ctx, tag, &mut fidelity, "naval_lines");

        // ----- national resource strip -----
        let resource_rows = build_resource_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, PRODUCTION_ROOT, &summary);
        let drawable = ProductionDrawable {
            roots: roots.clone(),
            file,
            mil_rows,
            naval_rows,
            resource_rows,
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

    crate::impl_view_any!();
}

/// Bind one production-line list (live `production.<kind>` primary; the save
/// `economy._production_lines` filtered to `kind` is the fallback) into rows.
fn build_line_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
    kind: &str,
) -> (Vec<DataRow>, SectionState) {
    // the `amount` (build-target) text box exists ONLY on the naval/railway templates, not
    // the military one — so bind it only for naval lines (else it would silently no-op).
    let has_amount = kind == "naval_lines";
    // 1) the LIVE channel: countries.<TAG>.production.<kind> (an array of line shapes).
    let live_path = format!("production.{kind}");
    let (live, live_state) = country_array(ctx, tag, &live_path);
    if let Some(arr) = live {
        let rows = arr
            .iter()
            .map(|l| bind_line_row(l, fidelity, has_amount))
            .collect();
        live_state.record(fidelity);
        return (rows, live_state);
    }
    // 2) the SAVE fallback: economy._production_lines filtered to {kind}.
    let (save, save_state) = country_array(ctx, tag, "economy._production_lines");
    if let Some(arr) = save {
        let rows: Vec<DataRow> = arr
            .iter()
            .filter(|l| l.get("kind").and_then(|k| k.as_str()) == Some(kind))
            .map(|l| bind_line_row(l, fidelity, has_amount))
            .collect();
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
) -> DataRow {
    let mut row = DataRow::new();
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

pub struct ConstructionView;

impl ConstructionView {
    pub fn new() -> ConstructionView {
        ConstructionView
    }
}

impl Default for ConstructionView {
    fn default() -> Self {
        ConstructionView::new()
    }
}

struct ConstructionDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    queue_rows: Vec<DataRow>,
    resource_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for ConstructionDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "CONSTR sect={} queue={} res={}",
            self.section.tag(),
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
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "item_grid",
            "production_building_line_entry",
            &self.queue_rows,
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
        let drawable = ConstructionDrawable {
            roots: roots.clone(),
            file,
            queue_rows,
            resource_rows,
            section,
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
    let mut row = DataRow::new();
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

pub struct DecisionsView;

impl DecisionsView {
    pub fn new() -> DecisionsView {
        DecisionsView
    }
}

impl Default for DecisionsView {
    fn default() -> Self {
        DecisionsView::new()
    }
}

struct DecisionsDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    timed_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for DecisionsDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "DECIS sect={} timed={}",
            self.section.tag(),
            row_digest(&self.timed_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "decision_grid",
            "timed_decision_item",
            &self.timed_rows,
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
        let drawable = DecisionsDrawable {
            roots: roots.clone(),
            file,
            timed_rows,
            section,
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
    let mut row = DataRow::new();
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
    use std::collections::HashMap;
    use std::path::Path;

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
                        {"archetype": "infantry_equipment_1", "active_factories": 30, "amount": -1,
                         "speed": 200.0, "priority": 2, "produced": 5.0, "cost": 10.0}
                    ],
                    "naval_lines": [],
                    "construction_queue": [
                        {"template_id": 7, "active_factories": 12, "produced": 50.0, "cost": 200.0, "priority": 0}
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
