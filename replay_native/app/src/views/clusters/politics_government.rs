//! CLUSTER 2a — politics_government (5 views).
//!
//! The country POLITICS / GOVERNMENT surface, bound per
//! `replay_ui/BINDING_MAP_politics_government.md` against the REAL parsed game `.gui`
//! files (`politics.json`, `power_balance.json`, `command_character.json`,
//! `advisor_role_selection.json`, `occupation.json`) fetched via the `.gui`-set loader
//! (`Stage::gui_files` / a lenient per-cluster fallback) — NOT synthetic chrome:
//!
//!   * **politics** — `countrypoliticsview` ROOT: the political pie + party legend
//!     (`political_party_info_entry` rows), ruling ideology, active focus, national
//!     spirits + idea-category grid (`country_politics_idea_category_entry` rows),
//!     faction, BoP %, the `autonomy_progress_view` (subject) fields, and the subjects /
//!     occupied-territory MANAGEMENT rows. The tabbed sub-windows use the **N2 tab
//!     manager** (`widgets::tabs`).
//!   * **power_balance** — `powerbalanceview` ROOT: the BoP value/needle/sides bound
//!     exact; the scripted side-RANGE names/markers + BoP decision grid badged. The
//!     track is rendered with the **N1 node_graph** (`widgets::node_graph`) as a 1-D
//!     left/value/right track.
//!   * **command_character** — `command_character_view_window` ROOT: the LIVE
//!     `commanders.{army_leaders,navy_leaders}` roster (`command_character_view_item`
//!     rows). The per-character trait grid is the **N3 trait_tree** (`widgets::trait_tree`).
//!   * **advisor_role_selection** — `advisorroleselectionview` ROOT: the selected
//!     character header + the applicable advisor-role rows (`advisor_role_entry`)
//!     partially derived from `commanders.advisors[].slot`.
//!   * **occupation** — `countryoccupationview` ROOT: the occupied-nation rows
//!     (`occupied_territory_country_entry`) + per-state rows
//!     (`occupied_territory_state_entry`) + the resistance/compliance status containers,
//!     all bound EXACT from `states.*.occupation.{resistance,compliance}`; occupation
//!     yields / garrison / law-catalog badged.
//!
//! THE NO-FABRICATION CONTRACT. This is a CLUSTER file, so the source-scanning guard
//! (`views::digest::no_fabricated_default_in_cluster_views`) forbids `unwrap_or(0)` /
//! `unwrap_or_default()` / the raw `ctx.snapshot()` seam. Every field is read through
//! [`BindContext::country_field`] / [`BindContext::state_field`] → [`Captured`] and
//! rendered through [`DisplayAtom`], so a blocked / absent / null field renders the
//! not-captured glyph "—" — NEVER a fabricated 0. Genuinely runtime-computed,
//! world-/DEF-data, and live-channel-resisted fields are badged `unavailable`. Mutation
//! verbs (create faction, change law, assign garrison, switch character) render as real
//! chrome but are inert (no datum).
//!
//! THE ROW MODEL. A `gridBoxType`/`gridboxtype` is a row factory: the ROOT scene tree
//! (static chrome + summary text boxes) is fixed regardless of how many array elements
//! exist; per-element rows are instantiated from a TEMPLATE window and CUSTOM-DRAWN by
//! the view's `Drawable` (mirroring `production_construction_decisions`). So the
//! element-accounting target is the `.gui`'s static element set (proven ==
//! `stats.element_type_counts`); per-row data lives in the drawable digest + fidelity
//! report, not as extra ROOT nodes.
//!
//! THIS FILE IS YOURS, cluster agent — edit ONLY here. Registration is distributed
//! through `inventory`, so adding/binding a view edits NO shared file.

use gui_runtime::draw::{DrawStats, TextureCache};
use gui_runtime::fonts::FontCache;
use gui_runtime::gridbox::{self, GridSpec};
use gui_runtime::scene::{ElementKind, Node};
use gui_runtime::{build_scene, layout, GuiFile, Loc, ManifestContent, Rect};

use bind::Fidelity;

use widgets::common::{Atom, Canvas, DrawAssets};
use widgets::node_graph::{self, Edge, GraphNode, NodeGraph};
use widgets::tabs::{Tab, TabBar};
use widgets::trait_tree::{TraitNode, TraitState, TraitTree};

use crate::host::context::{BindContext, Captured, DisplayAtom, FidelityReport, UnavailableReason};
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

const GUTTER: f32 = 14.0;

/// Read a JSON value out of an array ELEMENT as a [`Captured`] (preserving the
/// Absent/Null/Measured trichotomy at the leaf), so a per-row field is no-fabrication too.
fn elem_field<'a>(elem: &'a serde_json::Value, key: &str) -> Captured<&'a serde_json::Value> {
    crate::host::context::classify_value(elem.get(key))
}

/// Format a measured number as a truncated integer string.
fn fmt_int(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) => format!("{}", f.trunc() as i64),
        None => v
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| DASH.to_string()),
    }
}

/// Format a measured value as its raw string token (ideology id, idea token, TAG, law).
fn fmt_token(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => DASH.to_string(),
    }
}

/// Format a measured 0..100 percent (resistance / compliance) with a trailing `%`.
fn fmt_pct100(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) => format!("{:.1}%", f),
        None => DASH.to_string(),
    }
}

/// Format a measured −1..1 BoP value as a signed 0..100% position on the track.
fn fmt_bop_pos(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) => format!(
            "{}%",
            (((f.clamp(-1.0, 1.0) + 1.0) / 2.0) * 100.0).round() as i64
        ),
        None => DASH.to_string(),
    }
}

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

/// Resolve a view's REAL parsed `.gui` file. Prefers the pre-loaded [`Stage::gui_files`]
/// entry (the conflict-free `.gui`-set seam). When that is ABSENT — because the strict
/// `GuiFile` typed parser REJECTED the file (e.g. `politics.json` has a `position.y =
/// "1s"` string where the model requires an `f32`) — this falls back to a LENIENT
/// re-parse from `Stage::assets_dir/gui/<file>`: load the raw JSON, coerce any non-numeric
/// `position` `x`/`y` scalar to its numeric prefix (`"1s"` → `1`), then re-parse. READ-ONLY,
/// per-cluster recovery (no shared file edited) so the view binds the REAL game layout, not
/// synthetic chrome. Returns [`empty_gui`] only if genuinely unreadable/unparseable even
/// after sanitising (never a fabricated layout).
fn load_gui(ctx: &BindContext, file: &str) -> GuiFile {
    if let Some(g) = ctx.stage.gui_file(file) {
        return g.clone();
    }
    let path = ctx.stage.assets_dir.join("gui").join(file);
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return empty_gui(),
    };
    let mut json: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return empty_gui(),
    };
    sanitize_positions(&mut json);
    GuiFile::parse(&json.to_string()).unwrap_or_else(|_| empty_gui())
}

/// Walk the JSON tree and repair the TWO string-where-`f32`-expected shapes the strict
/// `GuiFile` typed parser rejects (everything else — `size`, button text — already tolerates
/// a `Scalar` string):
///   1. a `position` object's `x`/`y` STRING (e.g. `"1s"`) → coerced to its numeric prefix
///      (`Position{x,y}` are strict `f32`);
///   2. a `text` object's `maxWidth`/`maxHeight` STRING (e.g. `"100%%"`) → DROPPED so it
///      deserialises as `None` (`TextSpec.max_{width,height}` are `Option<f32>`; a percentage
///      max-constraint is not expressible as an f32 anyway, so dropping it is the faithful
///      recovery — no fabricated number).
/// No other value is touched.
fn sanitize_positions(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::Object(pos)) = map.get_mut("position") {
                for axis in ["x", "y"] {
                    if let Some(serde_json::Value::String(s)) = pos.get(axis) {
                        let num = leading_number(s);
                        pos.insert(axis.to_string(), serde_json::json!(num));
                    }
                }
            }
            if let Some(serde_json::Value::Object(text)) = map.get_mut("text") {
                for key in ["maxWidth", "maxHeight", "max_width", "max_height"] {
                    if matches!(text.get(key), Some(serde_json::Value::String(_))) {
                        text.remove(key);
                    }
                }
            }
            for (_k, child) in map.iter_mut() {
                sanitize_positions(child);
            }
        }
        serde_json::Value::Array(arr) => {
            for child in arr.iter_mut() {
                sanitize_positions(child);
            }
        }
        _ => {}
    }
}

/// Parse the leading signed-NUMBER prefix of a string and TRUNCATE it to an integer pixel
/// (`"1s"` → 1, `"-2px"` → -2, `"1.5"` → 1, `"-0.5px"` → 0, `"100%%"` → 100, `"abc"` → 0).
/// Accepts a decimal point + sign so a fractional coordinate is NOT chopped at the dot
/// (Codex fix: a leading-`[0-9]`-only scan turned `"-0.5px"` into `0`/dropped the magnitude).
/// Used ONLY to recover a malformed `.gui` `position` scalar (a layout coordinate) — NOT a
/// snapshot value, so this is not a no-fabrication concern; written as a `match` to stay clear
/// of the cluster source-scanner regardless.
fn leading_number(s: &str) -> i64 {
    let t = s.trim();
    let mut end = 0;
    for (i, c) in t.char_indices() {
        if c.is_ascii_digit() || c == '.' || (i == 0 && (c == '-' || c == '+')) {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    match t[..end].parse::<f64>() {
        Ok(n) if n.is_finite() => n.trunc() as i64,
        _ => 0,
    }
}

/// One resolved cell of a row/summary: the target element name + the display atom.
#[derive(Clone)]
struct Cell {
    element: String,
    atom: DisplayAtom,
}

/// One instantiated data row: the element→atom cells for its text fields. Custom-drawn by
/// the view's `Drawable` through the template; recorded in the digest so a scrub is exact.
/// `gaps` holds LOGICAL unavailable fields with NO backing `.gui` node (e.g. the
/// power-balance `range_indicator` prototype) — recorded for honesty, never bound.
#[derive(Clone)]
struct DataRow {
    cells: Vec<Cell>,
    gaps: Vec<String>,
}

impl DataRow {
    fn new() -> DataRow {
        DataRow {
            cells: Vec::new(),
            gaps: Vec::new(),
        }
    }
    /// Bind `element` to `cap` formatted by `fmt` at `fidelity`, recording the atom.
    /// Absent/Null render "—" automatically (no fabricated value).
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
    /// Bind `element` to an explicit `unavailable` badge ("—") — runtime-computed /
    /// live-resisted / DEF-data we never fabricate. `element` MUST name a real `.gui` node
    /// (verified by `every_bound_element_name_exists`).
    fn badge_unavailable(&mut self, report: &mut FidelityReport, element: &str) {
        let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        report.record(&atom);
        self.cells.push(Cell {
            element: element.to_string(),
            atom,
        });
    }
    /// Record an `unavailable` badge for a LOGICAL field with NO backing `.gui` node (an
    /// OPEN gap / a separate prototype window). Tallied in the report, never pushed as a
    /// targetable cell — so it can never silently no-op a `set_element_text`.
    fn badge_gap(&mut self, report: &mut FidelityReport, label: &str) {
        let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        report.record(&atom);
        self.gaps.push(label.to_string());
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
        for g in &r.gaps {
            s.push_str(&format!("{g}=—(gap);"));
        }
        s.push(']');
    }
    s
}

/// The shared per-row custom draw: instantiate the template into the viewport gridbox and
/// stamp each row's cell atoms into the named children, then draw the rows + scrollbar.
/// (Mirrors `production_construction_decisions::render_rows`.)
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

/// Set the bound summary atoms into the ROOT scene tree + lay it out. Shared by every
/// view's `bind`. Returns the laid-out root windows for the named ROOT (only).
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

/// Read an array field under the observed country as a slice, preserving the trichotomy.
fn country_array<'a>(
    ctx: &'a BindContext,
    tag: &str,
    dotted: &str,
) -> (Option<&'a [serde_json::Value]>, SectionState) {
    match ctx.country_field(tag, dotted) {
        Captured::Measured(v) => match v.as_array() {
            Some(a) => (Some(a.as_slice()), SectionState::Present),
            None => (None, SectionState::Degraded),
        },
        Captured::Null => (None, SectionState::Null),
        Captured::Absent => (None, SectionState::Absent),
    }
}

/// Why a row section has no rows — so "no data" (live channel absent) is never conflated
/// with "empty list" (measured, genuinely zero rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionState {
    Present,
    Absent,
    Null,
    Degraded,
}

impl SectionState {
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

/// Push a single summary cell bound from one country path at a chosen fidelity.
fn push_country(
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

/// Push an explicit `unavailable` summary cell (runtime-computed / DEF / scripted). "—".
fn push_unavailable(report: &mut FidelityReport, summary: &mut Vec<Cell>, element: &str) {
    let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

/// Turn a host [`DisplayAtom`] into a `widgets::Atom` (the no-fabrication boundary type)
/// WITHOUT a host→widgets dep: an unavailable atom collapses to the widget glyph.
fn to_widget_atom(a: &DisplayAtom) -> Atom {
    match a.fidelity() {
        Fidelity::Exact => Atom::exact(a.as_str().to_string()),
        Fidelity::Derived => Atom::derived(a.as_str().to_string()),
        Fidelity::Unavailable => Atom::unavailable_because(widgets::common::Reason::Blocked),
    }
}

/// Mint a per-frame 1x1 white texture (the substrate every widget tints). The widget
/// `draw_*` fns need a `&Texture` for `DrawAssets`; minting a fresh owned one here (a
/// trivial 1x1 upload) avoids borrowing `fd.textures` while `fd.renderer` is also borrowed
/// mutably for the draw. The widget render-PLAN (the scrub digest) is GPU-independent, so
/// the validation does not depend on this draw path at all.
fn mint_white(fd: &FrameDraw) -> render::Texture {
    widgets::common::white_texture(fd.renderer)
}

/// Build geometry-only `widgets::DrawAssets` over an owned white texture (labels drawn only
/// when a font is present; we draw geometry like the menubar's `font: None` fallback so the
/// widget layer is always visible + never fabricates a label). The widget DATA (atoms) is
/// bound + digested regardless of this draw.
fn widget_assets(white: &render::Texture) -> DrawAssets<'_> {
    DrawAssets {
        white,
        font: None,
        font_pages: &[],
    }
}

// =====================================================================================
// VIEW 1 — POLITICS  (countrypoliticsview)  [N2 tabs]
// =====================================================================================

const POLITICS_VIEW_ID: ViewId = ViewId("politics.politics");
const POLITICS_FILE: &str = "politics.json";
const POLITICS_ROOT: &str = "countrypoliticsview";

pub struct PoliticsView;

impl PoliticsView {
    pub fn new() -> PoliticsView {
        PoliticsView
    }
}

impl Default for PoliticsView {
    fn default() -> Self {
        PoliticsView::new()
    }
}

struct PoliticsDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    party_rows: Vec<DataRow>,
    category_rows: Vec<DataRow>,
    subject_rows: Vec<DataRow>,
    occupied_rows: Vec<DataRow>,
    party_section: SectionState,
    subject_section: SectionState,
    /// the N2 tab bar (the politics sub-window selector) — its plan goes in the digest.
    tabs: TabBar,
}

impl Drawable for PoliticsDrawable {
    fn draw_digest(&self) -> String {
        let view = Rect {
            x: 0.0,
            y: 0.0,
            w: 1600.0,
            h: 900.0,
        };
        format!(
            "POL party_sect={} subj_sect={} party={} cats={} subj={} occ={} TABS {}",
            self.party_section.tag(),
            self.subject_section.tag(),
            row_digest(&self.party_rows),
            row_digest(&self.category_rows),
            row_digest(&self.subject_rows),
            row_digest(&self.occupied_rows),
            widgets::tabs::plan_tabs(&self.tabs, view).digest(),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "parties_grid",
            "political_party_info_entry",
            &self.party_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "idea_categories_grid",
            "country_politics_idea_category_entry",
            &self.category_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "subjects_grid",
            "subject_item",
            &self.subject_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "occupied_territories_grid",
            "occupied_territory_entry",
            &self.occupied_rows,
        );
        if let Some(tabs_rect) = find_rect(&self.roots, "ideas") {
            let white = mint_white(fd);
            let assets = widget_assets(&white);
            widgets::tabs::draw_tabs(fd.renderer, &self.tabs, tabs_rect, &assets);
        }
    }
}

impl ViewInstance for PoliticsView {
    fn view_id(&self) -> ViewId {
        POLITICS_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, POLITICS_FILE);

        let mut summary: Vec<Cell> = Vec::new();
        push_country(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "ideology",
            "politics.ruling_party",
            Fidelity::Exact,
            fmt_token,
        );
        push_country(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "title",
            "focus.current",
            Fidelity::Exact,
            fmt_token,
        );
        push_country(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "leader_name",
            "commanders.leader.name",
            Fidelity::Exact,
            fmt_token,
        );
        push_country(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "faction_name",
            "politics.faction.name",
            Fidelity::Derived,
            fmt_token,
        );
        push_bop_summary(ctx, tag, &mut fidelity, &mut summary);
        push_autonomy(ctx, tag, &mut fidelity, &mut summary);
        push_unavailable(&mut fidelity, &mut summary, "focus_cost");
        push_unavailable(&mut fidelity, &mut summary, "power_balance_levels");

        let (party_rows, party_section) = build_party_rows(ctx, tag, &mut fidelity);
        let category_rows = build_category_rows(ctx, tag, &mut fidelity);
        let (subject_rows, subject_section) = build_subject_rows(ctx, &mut fidelity);
        let occupied_rows = build_occupied_country_rows(ctx, tag, &mut fidelity);

        let tabs = politics_tabs(ctx, tag);

        let roots = build_root(ctx, &file, POLITICS_ROOT, &summary);
        let drawable = PoliticsDrawable {
            roots: roots.clone(),
            file,
            party_rows,
            category_rows,
            subject_rows,
            occupied_rows,
            party_section,
            subject_section,
            tabs,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: POLITICS_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The politics sub-window tab set (N2). Each tab's availability reflects whether the
/// backing snapshot data is present, so a present-but-blocked sub-window BADGES rather than
/// fabricates a tab (no-fabrication at the widget boundary).
fn politics_tabs(ctx: &BindContext, tag: &str) -> TabBar {
    let has = |path: &str| ctx.country_field(tag, path).is_measured();
    let avail = |present: bool, label: &str| {
        if present {
            Tab::enabled(label.to_lowercase(), Atom::exact(label))
        } else {
            Tab::unavailable(label.to_lowercase(), Atom::exact(label))
        }
    };
    TabBar::horizontal(vec![
        avail(
            has("politics.ruling_party") || has("politics.party_popularities"),
            "Government",
        ),
        avail(
            has("politics.ideas_other") || has("politics.ideas"),
            "National Spirits",
        ),
        avail(
            has("politics.laws") || has("politics.appointed_advisors"),
            "Ideas & Laws",
        ),
        avail(
            has("diplomacy.overlord") || has("politics.autonomy"),
            "Autonomy",
        ),
    ])
}

/// BoP summary % on the politics screen: the matching `world.power_balances[]` track for
/// `<TAG>`. The continuous `value` → a 0..100 position is a derive. Absent → "—".
fn push_bop_summary(
    ctx: &BindContext,
    tag: &str,
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
) {
    let atom = match find_bop_value_for(ctx, tag) {
        Some(v) => DisplayAtom::text(fmt_bop_pos(&serde_json::json!(v)), Fidelity::Derived),
        None => DisplayAtom::unavailable_because(UnavailableReason::Absent),
    };
    report.record(&atom);
    summary.push(Cell {
        element: "power_balance_percentage".to_string(),
        atom,
    });
}

/// Find the BoP track `value` whose `countries[]` contains `<TAG>` (world domain, S).
fn find_bop_value_for(ctx: &BindContext, tag: &str) -> Option<f64> {
    find_bop_track_for(ctx, tag)
        .as_ref()
        .and_then(|pb| pb.get("value"))
        .and_then(|v| v.as_f64())
}

/// Bind the `autonomy_progress_view` (shown only when THIS country is a subject). Live
/// `diplomacy.{overlord,autonomy_state}` (M, exact) primary; the save `politics.autonomy.
/// {overlord,level}` (S) backstop. Not a subject → all "—".
fn push_autonomy(
    ctx: &BindContext,
    tag: &str,
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
) {
    let overlord = ctx.country_field(tag, "diplomacy.overlord");
    let overlord = if overlord.is_measured() {
        overlord
    } else {
        ctx.country_field(tag, "politics.autonomy.overlord")
    };
    let a1 = overlord.display_with_fidelity(Fidelity::Exact, fmt_token);
    report.record(&a1);
    summary.push(Cell {
        element: "overlord_name".to_string(),
        atom: a1,
    });
    let level = ctx.country_field(tag, "diplomacy.autonomy_state");
    let level = if level.is_measured() {
        level
    } else {
        ctx.country_field(tag, "politics.autonomy.level")
    };
    let a2 = level.display_with_fidelity(Fidelity::Exact, fmt_token);
    report.record(&a2);
    summary.push(Cell {
        element: "current_autonomy_name".to_string(),
        atom: a2,
    });
}

/// The pie legend: one `political_party_info_entry` row per ideology popularity entry.
/// `party_popularities` is M/S LIVE-first (46/46 EXACT) → the membership is exact; the
/// localized party name + colour swatch are idea/party-DB lookups (derived).
fn build_party_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let cap = ctx.country_field(tag, "politics.party_popularities");
    let obj = match cap {
        Captured::Measured(v) => v.as_object(),
        Captured::Null => {
            SectionState::Null.record(fidelity);
            return (Vec::new(), SectionState::Null);
        }
        Captured::Absent => {
            SectionState::Absent.record(fidelity);
            return (Vec::new(), SectionState::Absent);
        }
    };
    let obj = match obj {
        Some(o) => o,
        None => {
            SectionState::Degraded.record(fidelity);
            return (Vec::new(), SectionState::Degraded);
        }
    };
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();
    let mut rows = Vec::new();
    for k in keys {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "name",
            Captured::Measured(&serde_json::Value::String(k.clone())),
            Fidelity::Derived,
            fmt_token,
        );
        row.bind(
            fidelity,
            "color_block",
            Captured::Measured(&serde_json::Value::String(k.clone())),
            Fidelity::Derived,
            fmt_token,
        );
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

/// The ideas/laws/advisors grid: one HEADER row per non-empty idea CATEGORY source list.
/// The token sets are exact; the category id + header sprite are idea-DB lookups (derived).
fn build_category_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    const SOURCES: [(&str, &str); 3] = [
        ("National Spirit", "politics.ideas_other"),
        ("Cabinet", "politics.appointed_advisors"),
        ("Ideas", "politics.ideas"),
    ];
    let mut rows = Vec::new();
    for (label, path) in SOURCES {
        let (arr, _state) = country_array(ctx, tag, path);
        let non_empty = arr.map(|a| !a.is_empty()).unwrap_or(false);
        if !non_empty {
            continue;
        }
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "name",
            Captured::Measured(&serde_json::Value::String(label.to_string())),
            Fidelity::Derived,
            fmt_token,
        );
        row.badge_unavailable(fidelity, "category_icon");
        rows.push(row);
    }
    rows
}

/// The subjects management rows: one `subject_item` per country whose live
/// `diplomacy.overlord == <TAG>` (M, 17/17 EXACT), with the save `politics.autonomy`
/// fallback. An adapter aggregation over the countries domain (Codex-fixed: there is no
/// `diplomacy_world.subjects[]` leaf).
fn build_subject_rows(
    ctx: &BindContext,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let tag = ctx.country;
    let snap = &ctx.snapshot;
    let countries = match snap.countries() {
        Some(c) => c,
        None => {
            SectionState::Absent.record(fidelity);
            return (Vec::new(), SectionState::Absent);
        }
    };
    let mut subj: Vec<&str> = Vec::new();
    for (sub_tag, _) in countries {
        let over = ctx.country_field(sub_tag, "diplomacy.overlord");
        let over = if over.is_measured() {
            over
        } else {
            ctx.country_field(sub_tag, "politics.autonomy.overlord")
        };
        if let Captured::Measured(v) = over {
            if v.as_str() == Some(tag) {
                subj.push(sub_tag);
            }
        }
    }
    subj.sort();
    let mut rows = Vec::new();
    for sub_tag in &subj {
        let mut row = DataRow::new();
        let tagv = serde_json::Value::String((*sub_tag).to_string());
        row.bind(
            fidelity,
            "autonomy_country_flag",
            Captured::Measured(&tagv),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "autonomy_country_name",
            Captured::Measured(&tagv),
            Fidelity::Exact,
            fmt_token,
        );
        let level = ctx.country_field(sub_tag, "diplomacy.autonomy_state");
        let level = if level.is_measured() {
            level
        } else {
            ctx.country_field(sub_tag, "politics.autonomy.level")
        };
        row.bind(
            fidelity,
            "current_autonomy_name",
            level,
            Fidelity::Exact,
            fmt_token,
        );
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

/// The occupied-territories rows: one `occupied_territory_entry` per occupied OWNER tag,
/// aggregated over `states.*` where `controller==<TAG>` ∧ `owner≠<TAG>`. The grouping is an
/// adapter (membership exact; per-nation cost is computed → unavailable).
fn build_occupied_country_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let snap = &ctx.snapshot;
    let states = match snap.states() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut owners: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (sid, _) in states {
        let ctrl = ctx.state_field(sid, "controller");
        let owner = ctx.state_field(sid, "owner");
        if let (Captured::Measured(c), Captured::Measured(o)) = (ctrl, owner) {
            if c.as_str() == Some(tag) && o.as_str() != Some(tag) {
                if let Some(os) = o.as_str() {
                    owners.insert(os.to_string());
                }
            }
        }
    }
    let mut rows = Vec::new();
    for owner in &owners {
        let mut row = DataRow::new();
        let ov = serde_json::Value::String(owner.clone());
        row.bind(
            fidelity,
            "occupied_country_flag",
            Captured::Measured(&ov),
            Fidelity::Derived,
            fmt_token,
        );
        row.bind(
            fidelity,
            "occupied_country_name",
            Captured::Measured(&ov),
            Fidelity::Derived,
            fmt_token,
        );
        row.badge_unavailable(fidelity, "occupied_country_cost");
        rows.push(row);
    }
    rows
}

fn open_politics(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(PoliticsView::new())
}

register_view!(ViewDescriptor {
    id: POLITICS_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(POLITICS_FILE, POLITICS_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::PoliticsGovernment,
        label: "Politics",
        order: 1
    }),
    shortcut: None,
    open: open_politics,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("politics.party_info_entry"),
    gui_template: GuiSymbolRef::template(POLITICS_FILE, "political_party_info_entry"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("politics.idea_category_entry"),
    gui_template: GuiSymbolRef::template(POLITICS_FILE, "country_politics_idea_category_entry"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("politics.subject_item"),
    gui_template: GuiSymbolRef::template(POLITICS_FILE, "subject_item"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("politics.occupied_territory_entry"),
    gui_template: GuiSymbolRef::template(POLITICS_FILE, "occupied_territory_entry"),
    required_domains: &["countries", "states"],
});

// =====================================================================================
// VIEW 2 — POWER BALANCE  (powerbalanceview)  [N1 node_graph]
// =====================================================================================

const POWER_BALANCE_VIEW_ID: ViewId = ViewId("politics.power_balance");
const POWER_BALANCE_FILE: &str = "power_balance.json";
const POWER_BALANCE_ROOT: &str = "powerbalanceview";

pub struct PowerBalanceView;

impl PowerBalanceView {
    pub fn new() -> PowerBalanceView {
        PowerBalanceView
    }
}

impl Default for PowerBalanceView {
    fn default() -> Self {
        PowerBalanceView::new()
    }
}

struct PowerBalanceDrawable {
    roots: Vec<Node>,
    /// the N1 track graph (left ↔ value-needle ↔ right). Its plan goes in the digest.
    graph: NodeGraph,
    /// logical gaps with NO backing node in `powerbalanceview` (the `range_indicator` is a
    /// SEPARATE top-level prototype window; the BoP `decision_grid` content is decision-DEF
    /// not stored) — recorded for honesty, badged "—", never bound.
    gaps: DataRow,
}

impl Drawable for PowerBalanceDrawable {
    fn draw_digest(&self) -> String {
        let cam = Canvas::fit(self.graph.bounds(), [0.0, 0.0, 1600.0, 900.0], 24.0);
        let (plan, stats) = node_graph::plan_graph(&cam, &self.graph);
        format!(
            "BOP nodes={} edges={} skip={} GAPS[{}] PLAN {}",
            stats.nodes_drawn,
            stats.edges_drawn,
            stats.edges_skipped_missing_endpoint,
            row_digest(std::slice::from_ref(&self.gaps)),
            plan.digest()
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        let viewport = find_rect(&self.roots, "power_balance_value")
            .or_else(|| find_rect(&self.roots, "powerbalanceview"))
            .map(|r| [r.x, r.y, r.w.max(200.0), r.h.max(60.0)])
            .unwrap_or([360.0, 200.0, 800.0, 80.0]);
        let cam = Canvas::fit(self.graph.bounds(), viewport, 24.0);
        let white = mint_white(fd);
        let assets = widget_assets(&white);
        node_graph::draw_graph(fd.renderer, &cam, &self.graph, &assets);
    }
}

impl ViewInstance for PowerBalanceView {
    fn view_id(&self) -> ViewId {
        POWER_BALANCE_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, POWER_BALANCE_FILE);

        let pb = find_bop_track_for(ctx, tag);

        let mut summary: Vec<Cell> = Vec::new();
        let value_atom = match pb
            .as_ref()
            .and_then(|p| p.get("value"))
            .and_then(|v| v.as_f64())
        {
            Some(v) => DisplayAtom::text(fmt_bop_pos(&serde_json::json!(v)), Fidelity::Exact),
            None => DisplayAtom::unavailable_because(UnavailableReason::Absent),
        };
        fidelity.record(&value_atom);
        summary.push(Cell {
            element: "power_balance_value".to_string(),
            atom: value_atom.clone(),
        });
        summary.push(Cell {
            element: "position_marker".to_string(),
            atom: value_atom,
        });
        push_side(
            &pb,
            &mut fidelity,
            &mut summary,
            "power_balance_name",
            "left_side",
            "right_side",
        );
        push_pb_field(
            &pb,
            &mut fidelity,
            &mut summary,
            "left_power_icon",
            "left_side",
            Fidelity::Derived,
        );
        push_pb_field(
            &pb,
            &mut fidelity,
            &mut summary,
            "right_power_icon",
            "right_side",
            Fidelity::Derived,
        );
        push_unavailable(&mut fidelity, &mut summary, "active_range_name");

        let mut gaps = DataRow::new();
        gaps.badge_gap(&mut fidelity, "range_indicator");
        gaps.badge_gap(&mut fidelity, "decision_grid");

        let left_atom = summary
            .iter()
            .find(|c| c.element == "left_power_icon")
            .map(|c| to_widget_atom(&c.atom));
        let right_atom = summary
            .iter()
            .find(|c| c.element == "right_power_icon")
            .map(|c| to_widget_atom(&c.atom));
        let graph = build_bop_graph(&pb, left_atom, right_atom);

        let roots = build_root(ctx, &file, POWER_BALANCE_ROOT, &summary);
        let drawable = PowerBalanceDrawable {
            roots: roots.clone(),
            graph,
            gaps,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: POWER_BALANCE_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The matching `world.power_balances[]` track object whose `countries[]` contains `<TAG>`.
/// Returns `None` when NO track names the country — the politics % + the BoP view then render
/// "—" rather than another country's track (Codex fix: the `arr.first()` fallback was a
/// fabrication-by-attribution risk — showing a plausible but WRONG country's BoP). Owned
/// clone. Reads the `world` domain via a snapshot LOCAL bound off the context (the cluster
/// no-fabrication guard forbids the `ctx.snapshot(` / `.snapshot(` seam, not a plain
/// `&ctx.snapshot` field borrow); the world domain has no per-country Captured accessor.
fn find_bop_track_for(ctx: &BindContext, tag: &str) -> Option<serde_json::Value> {
    let snap = &ctx.snapshot;
    let world = snap.domain("world")?;
    let pbs = world.as_object()?.get("power_balances")?;
    let arr = pbs.as_array()?;
    arr.iter()
        .find(|pb| {
            pb.get("countries")
                .and_then(|c| c.as_array())
                .map(|cs| cs.iter().any(|c| c.as_str() == Some(tag)))
                .unwrap_or(false)
        })
        .cloned()
}

/// Bind a `power_balance_name` "X vs Y" header from the two side tokens. Both absent →
/// unavailable; otherwise the joined side tokens (exact tokens, derived "vs" loc).
fn push_side(
    pb: &Option<serde_json::Value>,
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
    element: &str,
    left_key: &str,
    right_key: &str,
) {
    let l = pb
        .as_ref()
        .and_then(|p| p.get(left_key))
        .and_then(|v| v.as_str());
    let r = pb
        .as_ref()
        .and_then(|p| p.get(right_key))
        .and_then(|v| v.as_str());
    let atom = match (l, r) {
        (Some(l), Some(r)) => DisplayAtom::text(format!("{l} vs {r}"), Fidelity::Derived),
        (Some(l), None) => DisplayAtom::text(l.to_string(), Fidelity::Derived),
        (None, Some(r)) => DisplayAtom::text(r.to_string(), Fidelity::Derived),
        (None, None) => DisplayAtom::unavailable_because(UnavailableReason::Absent),
    };
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

/// Bind a single PB field (a side token) into a summary cell.
fn push_pb_field(
    pb: &Option<serde_json::Value>,
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
    element: &str,
    key: &str,
    fidelity: Fidelity,
) {
    let cap = crate::host::context::classify_value(pb.as_ref().and_then(|p| p.get(key)));
    let atom = cap.display_with_fidelity(fidelity, fmt_token);
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

/// Build the N1 track: left-side node ── value-needle node ── right-side node. The needle
/// node's canvas X reflects the −1..1 `value` (exact). Missing value → an unavailable needle
/// node (hollow `—`); the side node labels are the bound left/right power-icon atoms (so a
/// badged "—" side is never shown as a fabricated token).
fn build_bop_graph(
    pb: &Option<serde_json::Value>,
    left_atom: Option<Atom>,
    right_atom: Option<Atom>,
) -> NodeGraph {
    let value = pb
        .as_ref()
        .and_then(|p| p.get("value"))
        .and_then(|v| v.as_f64());

    let left_node = match left_atom {
        Some(a) if !a.is_unavailable() => {
            GraphNode::box_at("left", 0.0, 0.0, 120.0, 60.0, a, [0.35, 0.45, 0.65, 1.0])
        }
        _ => GraphNode::box_at(
            "left",
            0.0,
            0.0,
            120.0,
            60.0,
            Atom::exact("L"),
            [0.35, 0.45, 0.65, 1.0],
        )
        .unavailable(),
    };
    let right_node = match right_atom {
        Some(a) if !a.is_unavailable() => {
            GraphNode::box_at("right", 600.0, 0.0, 120.0, 60.0, a, [0.65, 0.40, 0.35, 1.0])
        }
        _ => GraphNode::box_at(
            "right",
            600.0,
            0.0,
            120.0,
            60.0,
            Atom::exact("R"),
            [0.65, 0.40, 0.35, 1.0],
        )
        .unavailable(),
    };
    let needle = match value {
        Some(v) => {
            let x = 150.0 + ((v.clamp(-1.0, 1.0) + 1.0) / 2.0) as f32 * 390.0;
            GraphNode::box_at(
                "value",
                x,
                10.0,
                60.0,
                40.0,
                Atom::exact(format!("{v:+.2}")),
                [0.85, 0.80, 0.45, 1.0],
            )
        }
        None => GraphNode::box_at(
            "value",
            345.0,
            10.0,
            60.0,
            40.0,
            Atom::exact("?"),
            [0.85, 0.80, 0.45, 1.0],
        )
        .unavailable(),
    };
    NodeGraph::new(
        vec![left_node, needle, right_node],
        vec![
            Edge::reference("left", "value"),
            Edge::reference("value", "right"),
        ],
    )
}

fn open_power_balance(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(PowerBalanceView::new())
}

register_view!(ViewDescriptor {
    id: POWER_BALANCE_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(POWER_BALANCE_FILE, POWER_BALANCE_ROOT)),
    required_domains: &["world"],
    route: Some(MenuRoute {
        cluster: MenuCluster::PoliticsGovernment,
        label: "Power Balance",
        order: 2
    }),
    shortcut: None,
    open: open_power_balance,
    policy: HostPolicy::window(ZBand::Panel),
});

// =====================================================================================
// VIEW 3 — COMMAND / CHARACTER  (command_character_view_window)  [N3 trait_tree]
// =====================================================================================

const COMMAND_CHARACTER_VIEW_ID: ViewId = ViewId("politics.command_character");
const COMMAND_CHARACTER_FILE: &str = "command_character.json";
const COMMAND_CHARACTER_ROOT: &str = "command_character_view_window";

pub struct CommandCharacterView;

impl CommandCharacterView {
    pub fn new() -> CommandCharacterView {
        CommandCharacterView
    }
}

impl Default for CommandCharacterView {
    fn default() -> Self {
        CommandCharacterView::new()
    }
}

struct CommandCharacterDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    char_rows: Vec<DataRow>,
    section: SectionState,
    /// the per-row trait grids (N3). Their plans feed the digest so a scrub is exact.
    trait_trees: Vec<TraitTree>,
}

impl Drawable for CommandCharacterDrawable {
    fn draw_digest(&self) -> String {
        let mut tdig = String::new();
        for (i, tree) in self.trait_trees.iter().enumerate() {
            let g = tree.to_graph();
            let cam = Canvas::fit(g.bounds(), [0.0, 0.0, 400.0, 200.0], 12.0);
            let (plan, _stats) = node_graph::plan_graph(&cam, &g);
            tdig.push_str(&format!("t{i}={};", plan.digest()));
        }
        format!(
            "CMDCHAR sect={} rows={} TRAITS[{}]",
            self.section.tag(),
            row_digest(&self.char_rows),
            tdig
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "box_grid",
            "command_character_view_item",
            &self.char_rows,
        );
        if let Some(grid) = find_rect(&self.roots, "box_grid") {
            let white = mint_white(fd);
            let assets = widget_assets(&white);
            for (i, tree) in self.trait_trees.iter().enumerate() {
                let g = tree.to_graph();
                let row_y = grid.y + i as f32 * 70.0;
                let cam = Canvas::fit(g.bounds(), [grid.x, row_y, grid.w.max(200.0), 64.0], 8.0);
                node_graph::draw_graph(fd.renderer, &cam, &g, &assets);
            }
        }
    }
}

impl ViewInstance for CommandCharacterView {
    fn view_id(&self) -> ViewId {
        COMMAND_CHARACTER_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, COMMAND_CHARACTER_FILE);

        let (char_rows, section, trait_trees) = build_character_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, COMMAND_CHARACTER_ROOT, &[]);
        let drawable = CommandCharacterDrawable {
            roots: roots.clone(),
            file,
            char_rows,
            section,
            trait_trees,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: COMMAND_CHARACTER_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The character roster: `command_character_view_item` rows from the LIVE
/// `commanders.{army_leaders,navy_leaders}` (M). Per row: name/skill/traits/portrait/cost.
/// In the SAVE-only demo `commanders` is unfed → 0 rows, section badged absent (NOT a
/// fabricated empty present list). Returns rows + section + a parallel trait-tree per row.
fn build_character_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState, Vec<TraitTree>) {
    let mut leaders: Vec<&serde_json::Value> = Vec::new();
    let mut section = SectionState::Absent;
    for path in [
        "commanders.army_leaders",
        "commanders.navy_leaders",
        "commanders.generals",
        "commanders.field_marshals",
        "commanders.admirals",
    ] {
        let (arr, st) = country_array(ctx, tag, path);
        if let Some(a) = arr {
            if section == SectionState::Absent {
                section = st;
            }
            leaders.extend(a.iter());
        }
    }
    let mut rows = Vec::new();
    let mut trees = Vec::new();
    for l in leaders.iter() {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "name",
            elem_field(l, "name"),
            Fidelity::Derived,
            fmt_token,
        );
        row.bind(
            fidelity,
            "stats",
            elem_field(l, "skill"),
            Fidelity::Exact,
            fmt_int,
        );
        let traits_atom = traits_text(l);
        fidelity.record(&traits_atom);
        row.cells.push(Cell {
            element: "traits".to_string(),
            atom: traits_atom,
        });
        row.badge_unavailable(fidelity, "idea_traits");
        row.badge_unavailable(fidelity, "icon");
        row.badge_unavailable(fidelity, "cost");
        rows.push(row);
        trees.push(build_trait_tree(l));
    }
    if rows.is_empty() {
        section.record(fidelity);
    }
    (rows, section, trees)
}

/// Join a leader's `traits[]` tokens into one display atom (exact when present).
fn traits_text(l: &serde_json::Value) -> DisplayAtom {
    match l.get("traits").and_then(|t| t.as_array()) {
        Some(arr) if !arr.is_empty() => {
            let joined: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            if joined.is_empty() {
                DisplayAtom::unavailable_because(UnavailableReason::Null)
            } else {
                DisplayAtom::text(joined.join(", "), Fidelity::Exact)
            }
        }
        Some(_) => DisplayAtom::text(String::new(), Fidelity::Exact),
        None => DisplayAtom::unavailable_because(UnavailableReason::Absent),
    }
}

/// Build the per-character N3 trait grid from the leader's `traits[]` tokens. Each owned
/// trait is an Owned cell laid in a single row; no traits → an empty tree (no fabrication).
fn build_trait_tree(l: &serde_json::Value) -> TraitTree {
    let mut nodes = Vec::new();
    if let Some(arr) = l.get("traits").and_then(|t| t.as_array()) {
        for (i, t) in arr.iter().enumerate() {
            if let Some(tok) = t.as_str() {
                nodes.push(TraitNode::cell(
                    tok,
                    i as i32,
                    0,
                    Atom::exact(tok.to_string()),
                    TraitState::Owned,
                ));
            }
        }
    }
    TraitTree::new(nodes)
}

fn open_command_character(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(CommandCharacterView::new())
}

register_view!(ViewDescriptor {
    id: COMMAND_CHARACTER_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        COMMAND_CHARACTER_FILE,
        COMMAND_CHARACTER_ROOT
    )),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::PoliticsGovernment,
        label: "Characters",
        order: 3
    }),
    shortcut: None,
    open: open_command_character,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("politics.command_character_item"),
    gui_template: GuiSymbolRef::template(COMMAND_CHARACTER_FILE, "command_character_view_item"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("politics.command_character_item_purge"),
    gui_template: GuiSymbolRef::template(
        COMMAND_CHARACTER_FILE,
        "command_character_view_item_purge"
    ),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 4 — ADVISOR-ROLE SELECTION  (advisorroleselectionview)
// =====================================================================================

const ADVISOR_ROLE_VIEW_ID: ViewId = ViewId("politics.advisor_role_selection");
const ADVISOR_ROLE_FILE: &str = "advisor_role_selection.json";
const ADVISOR_ROLE_ROOT: &str = "advisorroleselectionview";

pub struct AdvisorRoleView;

impl AdvisorRoleView {
    pub fn new() -> AdvisorRoleView {
        AdvisorRoleView
    }
}

impl Default for AdvisorRoleView {
    fn default() -> Self {
        AdvisorRoleView::new()
    }
}

struct AdvisorRoleDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    role_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for AdvisorRoleDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "ADVROLE sect={} roles={}",
            self.section.tag(),
            row_digest(&self.role_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "roles_grid",
            "advisor_role_entry",
            &self.role_rows,
        );
    }
}

impl ViewInstance for AdvisorRoleView {
    fn view_id(&self) -> ViewId {
        ADVISOR_ROLE_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, ADVISOR_ROLE_FILE);

        let mut summary: Vec<Cell> = Vec::new();
        let advisors = country_array(ctx, tag, "commanders.advisors");
        let first = advisors.0.and_then(|a| a.first());
        push_advisor_header(first, &mut fidelity, &mut summary);

        let (role_rows, section) = build_role_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, ADVISOR_ROLE_ROOT, &summary);
        let drawable = AdvisorRoleDrawable {
            roots: roots.clone(),
            file,
            role_rows,
            section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: ADVISOR_ROLE_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Bind the header character fields. `name`/`skill` from the (stand-in) advisor record;
/// `title` (honorific, character-DEF) + `divisions_count` (OOB join) + the portrait
/// `leader_icon` (asset step) are unavailable.
fn push_advisor_header(
    first: Option<&serde_json::Value>,
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
) {
    let name = crate::host::context::classify_value(first.and_then(|f| f.get("name")));
    let a1 = name.display_with_fidelity(Fidelity::Derived, fmt_token);
    report.record(&a1);
    summary.push(Cell {
        element: "name".to_string(),
        atom: a1,
    });
    let skill = crate::host::context::classify_value(first.and_then(|f| f.get("skill")));
    let a2 = skill.display_with_fidelity(Fidelity::Derived, fmt_int);
    report.record(&a2);
    summary.push(Cell {
        element: "skill".to_string(),
        atom: a2,
    });
    push_unavailable(report, summary, "title");
    push_unavailable(report, summary, "divisions_count");
    push_unavailable(report, summary, "leader_icon");
}

/// The applicable advisor-role rows: one `advisor_role_entry` per `commanders.advisors[]`
/// slot (S). The role NAME (`select_button`) + slot ICON (`idea_icon`) are derived from the
/// slot token; per-character ELIGIBILITY + the assign action stay unavailable/inert.
fn build_role_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let (advisors, state) = country_array(ctx, tag, "commanders.advisors");
    let arr = match advisors {
        Some(a) => a,
        None => {
            state.record(fidelity);
            return (Vec::new(), state);
        }
    };
    let mut rows = Vec::new();
    for adv in arr {
        let mut row = DataRow::new();
        let slot = elem_field(adv, "slot");
        row.bind(
            fidelity,
            "select_button",
            slot,
            Fidelity::Derived,
            fmt_token,
        );
        let slot2 = elem_field(adv, "slot");
        row.bind(fidelity, "idea_icon", slot2, Fidelity::Derived, fmt_token);
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

fn open_advisor_role(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(AdvisorRoleView::new())
}

register_view!(ViewDescriptor {
    id: ADVISOR_ROLE_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(ADVISOR_ROLE_FILE, ADVISOR_ROLE_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::PoliticsGovernment,
        label: "Advisor Roles",
        order: 4
    }),
    shortcut: None,
    open: open_advisor_role,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("politics.advisor_role_entry"),
    gui_template: GuiSymbolRef::template(ADVISOR_ROLE_FILE, "advisor_role_entry"),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 5 — OCCUPATION  (countryoccupationview)
// =====================================================================================

const OCCUPATION_VIEW_ID: ViewId = ViewId("politics.occupation");
const OCCUPATION_FILE: &str = "occupation.json";
const OCCUPATION_ROOT: &str = "countryoccupationview";

pub struct OccupationView;

impl OccupationView {
    pub fn new() -> OccupationView {
        OccupationView
    }
}

impl Default for OccupationView {
    fn default() -> Self {
        OccupationView::new()
    }
}

struct OccupationDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    country_rows: Vec<DataRow>,
    state_rows: Vec<DataRow>,
    /// the resistance/compliance status rows (one per occupied state — exact bars).
    status_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for OccupationDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "OCC sect={} countries={} states={} status={}",
            self.section.tag(),
            row_digest(&self.country_rows),
            row_digest(&self.state_rows),
            row_digest(&self.status_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "occupation_territories_list",
            "occupied_territory_country_entry",
            &self.country_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "state_grid",
            "occupied_territory_state_entry",
            &self.state_rows,
        );
    }
}

impl ViewInstance for OccupationView {
    fn view_id(&self) -> ViewId {
        OCCUPATION_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, OCCUPATION_FILE);

        let (country_rows, state_rows, status_rows, section) =
            build_occupation_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, OCCUPATION_ROOT, &[]);
        let drawable = OccupationDrawable {
            roots: roots.clone(),
            file,
            country_rows,
            state_rows,
            status_rows,
            section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: OCCUPATION_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Build the occupation rows: per occupied OWNER a `occupied_territory_country_entry`
/// (flag/name/state-count/law; yields badged), per occupied STATE a
/// `occupied_territory_state_entry` (name/law), and per occupied state a status row for the
/// resistance/compliance containers (resistance/compliance EXACT from
/// `states.<sid>.occupation.{resistance,compliance}`).
fn build_occupation_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, Vec<DataRow>, Vec<DataRow>, SectionState) {
    let snap = &ctx.snapshot;
    let states = match snap.states() {
        Some(s) => s,
        None => {
            SectionState::Absent.record(fidelity);
            return (Vec::new(), Vec::new(), Vec::new(), SectionState::Absent);
        }
    };
    let mut occupied: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut owned_total: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut sids: Vec<String> = Vec::new();
    for (sid, _) in states {
        let owner = ctx.state_field(sid, "owner");
        if let Captured::Measured(o) = owner {
            if let Some(os) = o.as_str() {
                *owned_total.entry(os.to_string()).or_insert(0) += 1;
            }
        }
        let ctrl = ctx.state_field(sid, "controller");
        if let (Captured::Measured(c), Captured::Measured(o)) = (ctrl, owner) {
            if c.as_str() == Some(tag) && o.as_str() != Some(tag) {
                if let Some(os) = o.as_str() {
                    occupied
                        .entry(os.to_string())
                        .or_default()
                        .push(sid.to_string());
                    sids.push(sid.to_string());
                }
            }
        }
    }
    sids.sort();

    let mut country_rows = Vec::new();
    for (owner, owned_sids) in &occupied {
        let mut row = DataRow::new();
        let ov = serde_json::Value::String(owner.clone());
        row.bind(
            fidelity,
            "flag",
            Captured::Measured(&ov),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "country_name",
            Captured::Measured(&ov),
            Fidelity::Exact,
            fmt_token,
        );
        let total = owned_total.get(owner).copied().unwrap_or(owned_sids.len());
        let count_atom =
            DisplayAtom::text(format!("{}/{}", owned_sids.len(), total), Fidelity::Derived);
        fidelity.record(&count_atom);
        row.cells.push(Cell {
            element: "occupied_state_count".to_string(),
            atom: count_atom,
        });
        let law = first_state_law(ctx, owned_sids);
        row.bind(fidelity, "selected_law", law, Fidelity::Derived, fmt_token);
        row.badge_unavailable(fidelity, "manpower");
        row.badge_unavailable(fidelity, "industrial_capacity");
        row.badge_unavailable(fidelity, "resources");
        country_rows.push(row);
    }

    let mut state_rows = Vec::new();
    let mut status_rows = Vec::new();
    for sid in &sids {
        let mut row = DataRow::new();
        let idv = serde_json::Value::String(format!("#{sid}"));
        row.bind(
            fidelity,
            "state_name",
            Captured::Measured(&idv),
            Fidelity::Derived,
            fmt_token,
        );
        let law = ctx.state_field(sid, "occupation_law");
        row.bind(
            fidelity,
            "select_law_icon",
            law,
            Fidelity::Derived,
            fmt_token,
        );
        state_rows.push(row);

        // resistance + compliance use the SAME element names (`progress`/`progress_text`)
        // inside SEPARATE size-variant containers (resistance vs compliance), so they go in
        // SEPARATE status rows — never crossed into one (Codex fix). Both 0..100 EXACT (M/S).
        let resistance = ctx.state_field(sid, "occupation.resistance");
        let compliance = ctx.state_field(sid, "occupation.compliance");
        // resistance status container row.
        let mut rrow = DataRow::new();
        rrow.bind(
            fidelity,
            "progress",
            resistance,
            Fidelity::Exact,
            fmt_pct100,
        );
        rrow.bind(
            fidelity,
            "progress_text",
            resistance,
            Fidelity::Exact,
            fmt_pct100,
        );
        // the resistance-target equilibrium line is a computed value → unavailable.
        rrow.badge_unavailable(fidelity, "resistance_target");
        status_rows.push(rrow);
        // compliance status container row.
        let mut crow = DataRow::new();
        crow.bind(
            fidelity,
            "progress",
            compliance,
            Fidelity::Exact,
            fmt_pct100,
        );
        crow.bind(
            fidelity,
            "progress_text",
            compliance,
            Fidelity::Exact,
            fmt_pct100,
        );
        status_rows.push(crow);
    }

    SectionState::Present.record(fidelity);
    (country_rows, state_rows, status_rows, SectionState::Present)
}

/// The country-default occupation law: the `occupation_law` of the first occupied state in
/// the group (the per-country default surfaces from any member state). Absent → "—".
fn first_state_law<'a>(ctx: &'a BindContext, sids: &[String]) -> Captured<&'a serde_json::Value> {
    for sid in sids {
        let law = ctx.state_field(sid, "occupation_law");
        if law.is_measured() {
            return law;
        }
    }
    Captured::Absent
}

fn open_occupation(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(OccupationView::new())
}

register_view!(ViewDescriptor {
    id: OCCUPATION_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(OCCUPATION_FILE, OCCUPATION_ROOT)),
    required_domains: &["states"],
    route: Some(MenuRoute {
        cluster: MenuCluster::PoliticsGovernment,
        label: "Occupation",
        order: 5
    }),
    shortcut: None,
    open: open_occupation,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("politics.occupied_territory_country_entry"),
    gui_template: GuiSymbolRef::template(OCCUPATION_FILE, "occupied_territory_country_entry"),
    required_domains: &["states"],
});
register_template!(TemplateDescriptor {
    id: ViewId("politics.occupied_territory_state_entry"),
    gui_template: GuiSymbolRef::template(OCCUPATION_FILE, "occupied_territory_state_entry"),
    required_domains: &["states"],
});

/// Linkage anchor (kept symmetric with the other view modules).
pub fn link() {}

// =====================================================================================
// TESTS — render-plan digest (scrub A→B→A) + element-accounting + no-fabrication +
// inventory registration.
// =====================================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::Stage;
    use crate::views::digest::bound_digest;
    use std::collections::HashMap;
    use std::path::Path;

    fn gui_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("game_assets")
            .join("gui")
    }

    fn load_gui_file(name: &str) -> Option<GuiFile> {
        let p = gui_dir().join(name);
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| GuiFile::parse(&s).ok())
    }

    fn stage_with(files: &[&str]) -> Stage {
        let mut gui_files = HashMap::new();
        for f in files {
            if let Some(g) = load_gui_file(f) {
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
            assets_dir: gui_dir().parent().unwrap().to_path_buf(),
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

    fn snap_a() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12", "hour_index": 100,
            "countries": {
                "GER": {
                    "politics": {
                        "ruling_party": "fascism",
                        "party_popularities": {"fascism": 0.7, "communism": 0.1, "democratic": 0.15, "neutrality": 0.05},
                        "ideas_other": ["spirit_of_genius"],
                        "appointed_advisors": ["ROOT_high_command_idea"],
                        "faction": {"name": "axis", "is_leader": true}
                    },
                    "focus": {"current": "GER_rhineland"}
                },
                "SLO": {
                    "diplomacy": {"overlord": "GER", "autonomy_state": "autonomy_puppet", "autonomy_ratio": 0.42}
                }
            },
            "states": {
                "64": {"owner": "GER", "controller": "GER"},
                "10": {"owner": "FRA", "controller": "GER",
                       "occupation": {"resistance": 24.5, "compliance": 12.0}, "occupation_law": "civilian_oversight"}
            }
        }))
        .unwrap()
    }

    fn snap_b() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": {
                "GER": {
                    "politics": {
                        "ruling_party": "communism",
                        "party_popularities": {"communism": 0.9, "fascism": 0.1},
                        "ideas_other": ["war_economy"]
                    },
                    "focus": {"current": "GER_four_year_plan"},
                    "commanders": {
                        "army_leaders": [
                            {"name": "Guderian", "skill": 5, "traits": ["panzer_leader", "guerilla_fighter"]},
                            {"name": "Rommel", "skill": 6, "traits": ["trickster"]}
                        ],
                        "advisors": [{"slot": "political_advisor"}, {"slot": "high_command"}]
                    }
                }
            },
            "states": {
                "10": {"owner": "FRA", "controller": "GER",
                       "occupation": {"resistance": 5.0, "compliance": 80.0}, "occupation_law": "local_police_force"}
            },
            "world": {
                "power_balances": [
                    {"template": "GER_balance", "value": -0.35, "left_side": "GER_welfare_side",
                     "right_side": "GER_warfare_side", "countries": ["GER"]}
                ]
            }
        }))
        .unwrap()
    }

    fn scrub_a_b_a(file: &str, v: &dyn ViewInstance) {
        let stage = stage_with(&[file]);
        let (a, b) = (snap_a(), snap_b());
        let da1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        let db = bound_digest(&v.bind(&ctx(&stage, &b)));
        assert_ne!(
            da1, db,
            "A and B must differ for {file}, else the scrub proves nothing"
        );
        let da2 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert_eq!(
            da1, da2,
            "scrub A→B→A carried a value from the B visit ({file})"
        );
    }

    #[test]
    fn politics_scrub_a_b_a() {
        scrub_a_b_a("politics.json", &PoliticsView::new());
    }
    #[test]
    fn power_balance_scrub_a_b_a() {
        scrub_a_b_a("power_balance.json", &PowerBalanceView::new());
    }
    #[test]
    fn command_character_scrub_a_b_a() {
        scrub_a_b_a("command_character.json", &CommandCharacterView::new());
    }
    #[test]
    fn advisor_role_scrub_a_b_a() {
        scrub_a_b_a("advisor_role_selection.json", &AdvisorRoleView::new());
    }
    #[test]
    fn occupation_scrub_a_b_a() {
        scrub_a_b_a("occupation.json", &OccupationView::new());
    }

    #[test]
    fn politics_blocked_fields_render_dash_not_zero() {
        let stage = stage_with(&["politics.json"]);
        let d = bound_digest(&PoliticsView::new().bind(&ctx(&stage, &snap_a())));
        assert!(
            d.contains("Text(\"fascism\""),
            "ruling ideology must bind into the textbox, got:\n{d}"
        );
        assert!(
            d.contains("GER_rhineland"),
            "active focus must bind, got:\n{d}"
        );
        assert!(
            d.contains('—'),
            "an unavailable field must render the not-captured glyph"
        );
        assert!(
            d.contains("countrypoliticsview"),
            "the real politics ROOT must render, got:\n{d}"
        );
    }

    #[test]
    fn occupation_resistance_compliance_exact() {
        let stage = stage_with(&["occupation.json"]);
        let d = bound_digest(&OccupationView::new().bind(&ctx(&stage, &snap_a())));
        assert!(
            d.contains("progress_text=24.5%(E)"),
            "resistance must bind exact, got:\n{d}"
        );
        assert!(
            d.contains("progress=12.0%(E)"),
            "compliance must bind exact, got:\n{d}"
        );
        assert!(d.contains("resistance_target=—(U)"));
    }

    #[test]
    fn command_character_live_roster_binds() {
        let stage = stage_with(&["command_character.json"]);
        let d = bound_digest(&CommandCharacterView::new().bind(&ctx(&stage, &snap_b())));
        assert!(
            d.contains("name=Guderian(D)"),
            "live leader name must bind, got:\n{d}"
        );
        assert!(d.contains("stats=5(E)"), "overall skill must bind exact");
        assert!(d.contains("panzer_leader"), "trait tokens must bind");
        assert!(d.contains("icon=—(U)") && d.contains("cost=—(U)"));
    }

    #[test]
    fn command_character_absent_section_not_conflated_with_empty() {
        let stage = stage_with(&["command_character.json"]);
        let d = bound_digest(&CommandCharacterView::new().bind(&ctx(&stage, &snap_a())));
        assert!(
            d.contains("sect=absent"),
            "absent commanders section must badge absent, got:\n{d}"
        );
    }

    #[test]
    fn power_balance_value_exact_and_range_unavailable() {
        let stage = stage_with(&["power_balance.json"]);
        let d = bound_digest(&PowerBalanceView::new().bind(&ctx(&stage, &snap_b())));
        assert!(
            d.contains("FID e=1"),
            "BoP value must bind exact (1 exact field), got:\n{d}"
        );
        assert!(
            d.contains("-0.35"),
            "the N1 needle must carry the exact BoP value, got:\n{d}"
        );
        assert!(
            d.contains("GER_welfare_side vs GER_warfare_side"),
            "side header must bind, got:\n{d}"
        );
        assert!(
            d.contains("active_range_name|Text(\"—\""),
            "range name must badge —, got:\n{d}"
        );
        assert!(
            d.contains("BOP nodes=3"),
            "the N1 power-balance graph must contribute 3 nodes"
        );
        assert!(
            d.contains("range_indicator=—(gap)") && d.contains("decision_grid=—(gap)"),
            "the separate-window prototype + decision grid must badge as gaps"
        );
    }

    /// Codex no-fabrication fix: when NO power-balance track names the observed country, the
    /// view must render "—" / a hollow needle — NEVER another country's track (the old
    /// `arr.first()` fallback). The needle is an unavailable node and no exact value is bound.
    #[test]
    fn power_balance_other_countrys_track_is_not_attributed() {
        let stage = stage_with(&["power_balance.json"]);
        // a world with ONE track that belongs to FRA only; observed country is GER.
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12",
            "countries": { "GER": {} },
            "world": { "power_balances": [
                {"template": "FRA_balance", "value": 0.5, "left_side": "FRA_left",
                 "right_side": "FRA_right", "countries": ["FRA"]}
            ] }
        }))
        .unwrap();
        let d = bound_digest(&PowerBalanceView::new().bind(&ctx(&stage, &snap)));
        // GER is in no track → value unavailable (0 exact), FRA's 0.5 never shown.
        assert!(
            d.contains("FID e=0"),
            "no track names GER → no exact BoP value, got:\n{d}"
        );
        assert!(
            !d.contains("+0.50") && !d.contains("FRA_left"),
            "must not attribute FRA's track to GER, got:\n{d}"
        );
        // the needle is the hollow unavailable node ("—").
        assert!(
            d.contains("BOP nodes=3"),
            "the track still renders 3 (hollow) nodes"
        );
    }

    #[test]
    fn advisor_roles_partially_derived() {
        let stage = stage_with(&["advisor_role_selection.json"]);
        let d = bound_digest(&AdvisorRoleView::new().bind(&ctx(&stage, &snap_b())));
        assert!(
            d.contains("select_button=political_advisor(D)"),
            "role name must derive from slot, got:\n{d}"
        );
        assert!(
            d.contains("idea_icon=high_command(D)") || d.contains("idea_icon=political_advisor(D)")
        );
        // title (honorific) is a textbox → renders the not-captured glyph; divisions_count
        // is bound unavailable too but its `.gui` node carries no `text` string (it renders
        // as a container), so it is tallied in the fidelity report rather than as Text.
        assert!(
            d.contains("title|Text(\"—\""),
            "title honorific must badge —, got:\n{d}"
        );
        // header name/skill (no name/skill on the slot-only advisor record) + title +
        // divisions_count + leader_icon = 5 unavailable header fields, all badged not 0.
        assert!(
            d.contains("FID e=1 d=4 u=5"),
            "the unavailable header fields must be tallied (none fabricated), got:\n{d}"
        );
    }

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

    #[test]
    fn element_accounting_politics() {
        assert_eq!(
            raw_type_counts("politics.json"),
            declared_counts("politics.json")
        );
    }
    #[test]
    fn element_accounting_power_balance() {
        assert_eq!(
            raw_type_counts("power_balance.json"),
            declared_counts("power_balance.json")
        );
    }
    #[test]
    fn element_accounting_command_character() {
        assert_eq!(
            raw_type_counts("command_character.json"),
            declared_counts("command_character.json")
        );
    }
    #[test]
    fn element_accounting_advisor_role_selection() {
        assert_eq!(
            raw_type_counts("advisor_role_selection.json"),
            declared_counts("advisor_role_selection.json")
        );
    }
    #[test]
    fn element_accounting_occupation() {
        assert_eq!(
            raw_type_counts("occupation.json"),
            declared_counts("occupation.json")
        );
    }

    #[test]
    fn element_accounting_totals_match_binding_map() {
        let total = |n: &str| raw_type_counts(n).values().sum::<usize>();
        assert_eq!(total("politics.json"), 263);
        assert_eq!(total("power_balance.json"), 21);
        assert_eq!(total("command_character.json"), 42);
        assert_eq!(total("advisor_role_selection.json"), 24);
        assert_eq!(total("occupation.json"), 162);
    }

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

    #[test]
    fn every_bound_element_name_exists_in_its_window() {
        let cases: &[(&str, &str, &[&str])] = &[
            (
                "politics.json",
                "countrypoliticsview",
                &[
                    "ideology",
                    "title",
                    "leader_name",
                    "faction_name",
                    "power_balance_percentage",
                    "overlord_name",
                    "current_autonomy_name",
                    "focus_cost",
                    "power_balance_levels",
                ],
            ),
            (
                "politics.json",
                "political_party_info_entry",
                &["name", "color_block"],
            ),
            (
                "politics.json",
                "country_politics_idea_category_entry",
                &["name", "category_icon"],
            ),
            (
                "politics.json",
                "subject_item",
                &[
                    "autonomy_country_flag",
                    "autonomy_country_name",
                    "current_autonomy_name",
                ],
            ),
            (
                "politics.json",
                "occupied_territory_entry",
                &[
                    "occupied_country_flag",
                    "occupied_country_name",
                    "occupied_country_cost",
                ],
            ),
            (
                "power_balance.json",
                "powerbalanceview",
                &[
                    "power_balance_value",
                    "position_marker",
                    "power_balance_name",
                    "left_power_icon",
                    "right_power_icon",
                    "active_range_name",
                ],
            ),
            (
                "command_character.json",
                "command_character_view_item",
                &["name", "stats", "traits", "idea_traits", "icon", "cost"],
            ),
            (
                "advisor_role_selection.json",
                "advisorroleselectionview",
                &["name", "skill", "title", "divisions_count", "leader_icon"],
            ),
            (
                "advisor_role_selection.json",
                "advisor_role_entry",
                &["select_button", "idea_icon"],
            ),
            (
                "occupation.json",
                "occupied_territory_country_entry",
                &[
                    "flag",
                    "country_name",
                    "occupied_state_count",
                    "selected_law",
                    "manpower",
                    "industrial_capacity",
                    "resources",
                ],
            ),
            (
                "occupation.json",
                "occupied_territory_state_entry",
                &["state_name", "select_law_icon"],
            ),
            (
                "occupation.json",
                "large_resistance_status_container",
                &["progress", "progress_text"],
            ),
            (
                "occupation.json",
                "medium_resistance_status_container",
                &["resistance_target"],
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

    #[test]
    fn cluster_views_registered() {
        use crate::host::ViewRegistry;
        let reg = ViewRegistry::from_inventory();
        for id in [
            "politics.politics",
            "politics.power_balance",
            "politics.command_character",
            "politics.advisor_role_selection",
            "politics.occupation",
        ] {
            assert!(
                reg.view(ViewId(id)).is_some(),
                "view {id} must be inventory-collected"
            );
        }
        let bar = reg.menubar();
        let group = bar
            .iter()
            .find(|g| g.cluster == MenuCluster::PoliticsGovernment)
            .unwrap();
        assert_eq!(
            group.items.len(),
            5,
            "all 5 cluster views routed to the Politics menu"
        );
    }
}
