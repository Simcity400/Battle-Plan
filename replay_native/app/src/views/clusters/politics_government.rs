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
//!     exact, active range name derived from static BoP defs; the range marker and
//!     BoP decision grid stay badged. The
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

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

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
use crate::host::input::{PointerButton, ViewInput};
use crate::host::registry::{
    GuiSymbolRef, MenuCluster, MenuRoute, TemplateDescriptor, ViewDescriptor, ViewId,
};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InputOutcome, InstanceId, RebindContext,
    ViewCommand, ViewInstance, ZBand,
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

#[derive(Debug, Default)]
struct BopDefCatalog {
    balances: HashMap<String, BopDef>,
}

#[derive(Debug, Default)]
struct BopDef {
    ranges: Vec<BopRangeDef>,
}

#[derive(Debug, Clone)]
struct BopRangeDef {
    id: String,
    min: f64,
    max: f64,
}

static BOP_CATALOG: OnceLock<BopDefCatalog> = OnceLock::new();

fn bop_catalog() -> &'static BopDefCatalog {
    BOP_CATALOG.get_or_init(BopDefCatalog::load)
}

impl BopDefCatalog {
    fn load() -> BopDefCatalog {
        match Self::catalog_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|txt| Self::parse(&txt))
        {
            Some(c) => c,
            None => BopDefCatalog::default(),
        }
    }

    fn catalog_path() -> Option<PathBuf> {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let replay_native = crate_dir.parent()?;
        let repo = replay_native.parent()?;
        Some(repo.join("game_defs").join("out").join("catalogs.json"))
    }

    fn parse(txt: &str) -> Option<BopDefCatalog> {
        let v: serde_json::Value = serde_json::from_str(txt).ok()?;
        Self::from_value(&v)
    }

    fn from_value(v: &serde_json::Value) -> Option<BopDefCatalog> {
        let cats = v.get("catalogs")?.as_object()?;
        let pbs = cats.get("power_balances")?.as_object()?;
        let mut out = BopDefCatalog::default();
        for (id, body) in pbs {
            let mut ranges = Vec::new();
            if let Some(arr) = body
                .get("neutral_ranges")
                .and_then(serde_json::Value::as_array)
            {
                push_bop_ranges(&mut ranges, arr);
            }
            if let Some(sides) = body.get("sides").and_then(serde_json::Value::as_array) {
                for side in sides {
                    if let Some(arr) = side.get("ranges").and_then(serde_json::Value::as_array) {
                        push_bop_ranges(&mut ranges, arr);
                    }
                }
            }
            out.balances.insert(id.clone(), BopDef { ranges });
        }
        Some(out)
    }

    fn active_range_id(&self, template: &str, value: f64) -> Option<&str> {
        self.balances.get(template)?.ranges.iter().find_map(|r| {
            if r.contains(value) {
                Some(r.id.as_str())
            } else {
                None
            }
        })
    }
}

fn push_bop_ranges(out: &mut Vec<BopRangeDef>, ranges: &[serde_json::Value]) {
    for r in ranges {
        let id = match r.get("id").and_then(serde_json::Value::as_str) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        let min = match value_to_f64(r.get("min")) {
            Some(v) => v,
            None => continue,
        };
        let max = match value_to_f64(r.get("max")) {
            Some(v) => v,
            None => continue,
        };
        out.push(BopRangeDef { id, min, max });
    }
}

fn value_to_f64(v: Option<&serde_json::Value>) -> Option<f64> {
    match v? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

impl BopRangeDef {
    fn contains(&self, value: f64) -> bool {
        let lo = self.min.min(self.max);
        let hi = self.min.max(self.max);
        value >= lo && value <= hi
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
    key: Option<String>,
    cells: Vec<Cell>,
    gaps: Vec<String>,
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
        for g in &r.gaps {
            s.push_str(&format!("{g}=—(gap);"));
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

fn suppress_duplicate_keys(keys: &mut [Option<String>]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
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
    render_rows_selected(fd, panel, file, grid_name, template_name, rows, 0.0, None);
}

fn render_rows_selected(
    fd: &mut FrameDraw,
    panel: &[Node],
    file: &GuiFile,
    grid_name: &str,
    template_name: &str,
    rows: &[DataRow],
    scroll_y: f32,
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
        scroll_y,
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
    let spec = GridSpec::from_gridbox(&stub_grid(grid_name, viewport), &template, GUTTER);
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

fn rects_intersect(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
}

fn instantiated_row_child_rects(
    ctx: &BindContext,
    panel: &[Node],
    file: &GuiFile,
    grid_name: &str,
    template_name: &str,
    row_count: usize,
    scroll_y: f32,
    child_name: &str,
) -> Vec<Rect> {
    if row_count == 0 {
        return Vec::new();
    }
    let viewport = match find_rect(panel, grid_name) {
        Some(r) => r,
        None => return Vec::new(),
    };
    let template = match find_template(file, template_name) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let spec = GridSpec::from_gridbox(&stub_grid(grid_name, viewport), &template, GUTTER);
    let content = ManifestContent {
        manifest: &ctx.stage.manifest,
        measure: |_t, _f| None,
    };
    gridbox::instantiate(&template, row_count, viewport, &spec, scroll_y, &content)
        .rows
        .iter()
        .filter_map(|row| find_rect(std::slice::from_ref(row), child_name))
        .filter(|rect| rects_intersect(*rect, viewport))
        .collect()
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
const POLITICS_PARTY_ROW_H: f32 = 18.0;
const POLITICS_TAB_ROW_H: f32 = 100.0;

pub struct PoliticsView {
    tabs: TabBar,
    tab_states: PoliticsTabStates,
    tab_scroll_drag: Option<PoliticsScrollDrag>,
    screen: Rect,
}

impl PoliticsView {
    pub fn new() -> PoliticsView {
        PoliticsView {
            tabs: politics_tabs_for_flags(true, true, true, true),
            tab_states: PoliticsTabStates::default(),
            tab_scroll_drag: None,
            screen: default_politics_screen(),
        }
    }
}

impl Default for PoliticsView {
    fn default() -> Self {
        PoliticsView::new()
    }
}

#[derive(Default)]
struct PoliticsListState {
    selected_key: Option<String>,
    row_keys: Vec<Option<String>>,
    scroll_y: f32,
}

#[derive(Default)]
struct PoliticsTabStates {
    government: PoliticsListState,
    national_spirits: PoliticsListState,
    ideas_laws: PoliticsListState,
    autonomy: PoliticsListState,
}

struct PoliticsDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    tab: Option<PoliticsTab>,
    tab_rows: Vec<DataRow>,
    tab_section: SectionState,
    selected_key: Option<String>,
    list_window: PoliticsListWindow,
    /// the N2 tab bar (the politics sub-window selector) — its plan goes in the digest.
    tabs: TabBar,
    tab_panel: Rect,
}

impl Drawable for PoliticsDrawable {
    fn draw_digest(&self) -> String {
        let selected_state = match self.selected_key.as_deref() {
            None => "none",
            Some(key) if self.tab_rows.iter().any(|r| r.key.as_deref() == Some(key)) => "selected",
            Some(_) => "missing_selected",
        };
        format!(
            "POL tab={} sect={} selected={} selected_key={} selected_row={} scroll={:.0}/{:.0} visible=[{}] rows={} TABS {}",
            self.tabs.selected().unwrap_or("none"),
            self.tab_section.tag(),
            selected_state,
            self.selected_key.as_deref().unwrap_or("none"),
            selected_row_digest(&self.tab_rows, self.selected_key.as_deref()),
            self.list_window.scroll_y,
            self.list_window.max_scroll,
            visible_politics_row_digest(&self.tab_rows, self.list_window),
            row_digest(&self.tab_rows),
            widgets::tabs::plan_tabs(&self.tabs, self.tab_panel).digest(),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        if let Some(tab) = self.tab {
            render_rows_selected(
                fd,
                &self.roots,
                &self.file,
                tab.grid_name(),
                tab.template_name(),
                &self.tab_rows,
                self.list_window.scroll_y,
                self.selected_key.as_deref(),
            );
        }
        let white = mint_white(fd);
        let assets = widget_assets(&white);
        widgets::tabs::draw_tabs(fd.renderer, &self.tabs, self.tab_panel, &assets);
    }
}

impl ViewInstance for PoliticsView {
    fn view_id(&self) -> ViewId {
        POLITICS_VIEW_ID
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
        let next = politics_tabs_from_rebind(ctx, ctx.country);
        reconcile_politics_tabs(&mut self.tabs, next);
        for tab in [
            PoliticsTab::Government,
            PoliticsTab::NationalSpirits,
            PoliticsTab::IdeasLaws,
            PoliticsTab::Autonomy,
        ] {
            let keys = politics_tab_keys_from_rebind(ctx, ctx.country, tab);
            let viewport = politics_grid_fallback(self.screen, tab);
            let max_scroll = max_politics_scroll_fallback(viewport, keys.len(), tab.row_h());
            let state = self.tab_states.get_mut(tab);
            state.row_keys = keys;
            state.scroll_y = state.scroll_y.min(max_scroll);
        }
        if let Some(drag) = self.tab_scroll_drag {
            let active = selected_politics_tab(&self.tabs) == Some(drag.tab);
            let state = self.tab_states.get(drag.tab);
            let viewport = politics_grid_fallback(self.screen, drag.tab);
            if !active
                || politics_scrollbar_geometry(
                    viewport,
                    state.row_keys.len(),
                    drag.tab.row_h(),
                    state.scroll_y,
                )
                .is_none()
            {
                self.tab_scroll_drag = None;
            }
        }
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => {
                match click_politics_tab(&mut self.tabs, politics_tab_fallback(self.screen), x, y) {
                    PoliticsTabClick::Changed => return politics_input_consumed(true),
                    PoliticsTabClick::Unchanged | PoliticsTabClick::Inert => {
                        return politics_input_consumed(false);
                    }
                    PoliticsTabClick::Miss => {}
                }
                let tab = match selected_politics_tab(&self.tabs) {
                    Some(tab) => tab,
                    None => return InputOutcome::ignored(),
                };
                let viewport = politics_grid_fallback(self.screen, tab);
                let row_h = tab.row_h();
                let state = self.tab_states.get(tab);
                let scroll_y = state.scroll_y.min(max_politics_scroll_fallback(
                    viewport,
                    state.row_keys.len(),
                    row_h,
                ));
                if let Some(geom) =
                    politics_scrollbar_geometry(viewport, state.row_keys.len(), row_h, scroll_y)
                {
                    if geom.thumb.contains(x, y) {
                        self.tab_scroll_drag = Some(PoliticsScrollDrag {
                            tab,
                            pointer_offset_y: y - geom.thumb.y,
                        });
                        return politics_input_consumed(false);
                    }
                    if geom.track.contains(x, y) {
                        return politics_input_consumed(false);
                    }
                }
                if let Some(key) =
                    pick_politics_row_key(viewport, &state.row_keys, row_h, scroll_y, x, y)
                {
                    let state = self.tab_states.get_mut(tab);
                    let changed = state.selected_key.as_deref() != Some(key.as_str());
                    state.selected_key = Some(key);
                    politics_input_consumed(changed)
                } else {
                    InputOutcome::ignored()
                }
            }
            ViewInput::Wheel { x, y, dy } => {
                let tab = match selected_politics_tab(&self.tabs) {
                    Some(tab) => tab,
                    None => return InputOutcome::ignored(),
                };
                let viewport = politics_grid_fallback(self.screen, tab);
                if x < viewport.x
                    || x > viewport.x + viewport.w
                    || y < viewport.y
                    || y > viewport.y + viewport.h
                {
                    return InputOutcome::ignored();
                }
                let state = self.tab_states.get_mut(tab);
                let current = state.scroll_y;
                let next = scroll_politics_offset(
                    current,
                    viewport,
                    state.row_keys.len(),
                    tab.row_h(),
                    dy,
                );
                state.scroll_y = next;
                politics_input_consumed(next != current)
            }
            ViewInput::PointerMove { x: _, y, drag } if drag => {
                let drag = match self.tab_scroll_drag {
                    Some(drag) => drag,
                    None => return InputOutcome::ignored(),
                };
                let viewport = politics_grid_fallback(self.screen, drag.tab);
                let state = self.tab_states.get(drag.tab);
                let geom = match politics_scrollbar_geometry(
                    viewport,
                    state.row_keys.len(),
                    drag.tab.row_h(),
                    state.scroll_y,
                ) {
                    Some(geom) => geom,
                    None => {
                        self.tab_scroll_drag = None;
                        return InputOutcome::ignored();
                    }
                };
                let current = state.scroll_y;
                let next = politics_scroll_from_thumb_y(geom, y - drag.pointer_offset_y);
                self.tab_states.get_mut(drag.tab).scroll_y = next;
                politics_input_consumed(next != current)
            }
            ViewInput::PointerUp {
                button: PointerButton::Left,
                ..
            } => {
                if self.tab_scroll_drag.take().is_some() {
                    politics_input_consumed(false)
                } else {
                    InputOutcome::ignored()
                }
            }
            _ => InputOutcome::ignored(),
        }
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

        let mut tabs = politics_tabs(ctx, tag);
        if let Some(selected) = self.tabs.selected() {
            tabs.select(selected);
        }
        tabs.reconcile();
        let tab = selected_politics_tab(&tabs);
        let (tab_rows, tab_section) = build_politics_tab_rows(ctx, tag, tab, &mut fidelity);

        let roots = build_root(ctx, &file, POLITICS_ROOT, &summary);
        let (selected_key, list_window) = match tab {
            Some(tab) => {
                let state = self.tab_states.get(tab);
                let viewport =
                    politics_grid_viewport(&roots, tab, politics_grid_fallback(ctx.screen, tab));
                (
                    state.selected_key.clone(),
                    politics_list_window(viewport, tab_rows.len(), state.scroll_y, tab.row_h()),
                )
            }
            None => (None, PoliticsListWindow::default()),
        };
        let tab_panel =
            find_rect(&roots, "ideas").unwrap_or_else(|| politics_tab_fallback(ctx.screen));
        let drawable = PoliticsDrawable {
            roots: roots.clone(),
            file,
            tab,
            tab_rows,
            tab_section,
            selected_key,
            list_window,
            tabs,
            tab_panel,
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
    politics_tabs_from_bind(ctx, tag)
}

const POLITICS_TAB_GOVERNMENT: &str = "government";
const POLITICS_TAB_NATIONAL_SPIRITS: &str = "national_spirits";
const POLITICS_TAB_IDEAS_LAWS: &str = "ideas_laws";
const POLITICS_TAB_AUTONOMY: &str = "autonomy";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PoliticsTab {
    Government,
    NationalSpirits,
    IdeasLaws,
    Autonomy,
}

impl PoliticsTab {
    fn grid_name(self) -> &'static str {
        match self {
            PoliticsTab::Government => "parties_grid",
            PoliticsTab::NationalSpirits | PoliticsTab::IdeasLaws | PoliticsTab::Autonomy => {
                "idea_categories_grid"
            }
        }
    }
    fn template_name(self) -> &'static str {
        match self {
            PoliticsTab::Government => "political_party_info_entry",
            PoliticsTab::NationalSpirits | PoliticsTab::IdeasLaws | PoliticsTab::Autonomy => {
                "country_politics_idea_category_entry"
            }
        }
    }
    fn row_h(self) -> f32 {
        match self {
            PoliticsTab::Government => POLITICS_PARTY_ROW_H,
            PoliticsTab::NationalSpirits | PoliticsTab::IdeasLaws | PoliticsTab::Autonomy => {
                POLITICS_TAB_ROW_H
            }
        }
    }
}

impl PoliticsTabStates {
    fn get(&self, tab: PoliticsTab) -> &PoliticsListState {
        match tab {
            PoliticsTab::Government => &self.government,
            PoliticsTab::NationalSpirits => &self.national_spirits,
            PoliticsTab::IdeasLaws => &self.ideas_laws,
            PoliticsTab::Autonomy => &self.autonomy,
        }
    }
    fn get_mut(&mut self, tab: PoliticsTab) -> &mut PoliticsListState {
        match tab {
            PoliticsTab::Government => &mut self.government,
            PoliticsTab::NationalSpirits => &mut self.national_spirits,
            PoliticsTab::IdeasLaws => &mut self.ideas_laws,
            PoliticsTab::Autonomy => &mut self.autonomy,
        }
    }
}

#[derive(Clone, Copy)]
struct PoliticsListWindow {
    viewport: Rect,
    scroll_y: f32,
    max_scroll: f32,
    first: usize,
    visible: usize,
}

impl Default for PoliticsListWindow {
    fn default() -> Self {
        PoliticsListWindow {
            viewport: Rect::default(),
            scroll_y: 0.0,
            max_scroll: 0.0,
            first: 0,
            visible: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct PoliticsScrollDrag {
    tab: PoliticsTab,
    pointer_offset_y: f32,
}

#[derive(Clone, Copy)]
struct PoliticsScrollbarGeometry {
    track: Rect,
    thumb: Rect,
    max_scroll: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PoliticsTabClick {
    Changed,
    Unchanged,
    Inert,
    Miss,
}

fn politics_tabs_from_bind(ctx: &BindContext, tag: &str) -> TabBar {
    let has = |path: &str| ctx.country_field(tag, path).is_measured();
    politics_tabs_for_flags(
        has("politics.ruling_party") || has("politics.party_popularities"),
        has("politics.ideas_other"),
        has("politics.laws") || has("politics.appointed_advisors") || has("politics.ideas"),
        has("diplomacy.overlord") || has("politics.autonomy") || has_autonomy_rows_bind(ctx, tag),
    )
}

fn politics_tabs_from_rebind(ctx: &RebindContext, tag: &str) -> TabBar {
    let has = |path: &str| ctx.country_field(tag, path).is_measured();
    politics_tabs_for_flags(
        has("politics.ruling_party") || has("politics.party_popularities"),
        has("politics.ideas_other"),
        has("politics.laws") || has("politics.appointed_advisors") || has("politics.ideas"),
        has("diplomacy.overlord") || has("politics.autonomy") || has_autonomy_rows_rebind(ctx, tag),
    )
}

fn politics_tabs_for_flags(
    government: bool,
    national_spirits: bool,
    ideas_laws: bool,
    autonomy: bool,
) -> TabBar {
    let avail = |present: bool, label: &str| {
        if present {
            Tab::enabled(politics_tab_id_for_label(label), Atom::exact(label))
        } else {
            Tab::unavailable(politics_tab_id_for_label(label), Atom::exact(label))
        }
    };
    TabBar::horizontal(vec![
        avail(government, "Government"),
        avail(national_spirits, "National Spirits"),
        avail(ideas_laws, "Ideas & Laws"),
        avail(autonomy, "Autonomy"),
    ])
}

fn politics_tab_id_for_label(label: &str) -> &'static str {
    match label {
        "Government" => POLITICS_TAB_GOVERNMENT,
        "National Spirits" => POLITICS_TAB_NATIONAL_SPIRITS,
        "Ideas & Laws" => POLITICS_TAB_IDEAS_LAWS,
        "Autonomy" => POLITICS_TAB_AUTONOMY,
        _ => POLITICS_TAB_GOVERNMENT,
    }
}

fn selected_politics_tab(bar: &TabBar) -> Option<PoliticsTab> {
    match bar.selected() {
        Some(POLITICS_TAB_GOVERNMENT) => Some(PoliticsTab::Government),
        Some(POLITICS_TAB_NATIONAL_SPIRITS) => Some(PoliticsTab::NationalSpirits),
        Some(POLITICS_TAB_IDEAS_LAWS) => Some(PoliticsTab::IdeasLaws),
        Some(POLITICS_TAB_AUTONOMY) => Some(PoliticsTab::Autonomy),
        _ => None,
    }
}

fn reconcile_politics_tabs(current: &mut TabBar, next: TabBar) {
    let selected = current.selected().map(str::to_string);
    *current = next;
    if let Some(id) = selected {
        current.select(&id);
    }
    current.reconcile();
}

fn has_autonomy_rows_bind(ctx: &BindContext, tag: &str) -> bool {
    if let Some(countries) = ctx.countries() {
        for (sub_tag, _) in countries {
            if sub_tag == tag {
                continue;
            }
            let over = ctx.country_field(sub_tag, "diplomacy.overlord");
            let over = if over.is_measured() {
                over
            } else {
                ctx.country_field(sub_tag, "politics.autonomy.overlord")
            };
            if let Captured::Measured(v) = over {
                if v.as_str() == Some(tag) {
                    return true;
                }
            }
        }
    }
    if let Some(states) = ctx.states() {
        for (sid, _) in states {
            let ctrl = ctx.state_field(sid, "controller");
            let owner = ctx.state_field(sid, "owner");
            if let (Captured::Measured(c), Captured::Measured(o)) = (ctrl, owner) {
                if c.as_str() == Some(tag) && o.as_str() != Some(tag) {
                    return true;
                }
            }
        }
    }
    false
}

fn has_autonomy_rows_rebind(ctx: &RebindContext, tag: &str) -> bool {
    if let Some(countries) = ctx.countries() {
        for (sub_tag, _) in countries {
            if sub_tag == tag {
                continue;
            }
            let over = ctx.country_field(sub_tag, "diplomacy.overlord");
            let over = if over.is_measured() {
                over
            } else {
                ctx.country_field(sub_tag, "politics.autonomy.overlord")
            };
            if let Captured::Measured(v) = over {
                if v.as_str() == Some(tag) {
                    return true;
                }
            }
        }
    }
    if let Some(states) = ctx.states() {
        for (sid, _) in states {
            let ctrl = ctx.state_field(sid, "controller");
            let owner = ctx.state_field(sid, "owner");
            if let (Captured::Measured(c), Captured::Measured(o)) = (ctrl, owner) {
                if c.as_str() == Some(tag) && o.as_str() != Some(tag) {
                    return true;
                }
            }
        }
    }
    false
}

fn default_politics_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn politics_tab_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 606.0,
        y: screen.y + 78.0 + 425.0,
        w: 550.0,
        h: (screen.h - 78.0 - 425.0).max(widgets::tabs::BAR_THICK),
    }
}

fn politics_panel_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 606.0,
        y: screen.y + 78.0,
        w: 550.0,
        h: (screen.h - 78.0).max(0.0),
    }
}

fn politics_grid_fallback(screen: Rect, tab: PoliticsTab) -> Rect {
    match tab {
        PoliticsTab::Government => {
            let panel = politics_panel_fallback(screen);
            Rect {
                x: panel.x + 278.0,
                y: panel.y + 300.0,
                w: 330.0,
                h: 70.0,
            }
        }
        PoliticsTab::NationalSpirits | PoliticsTab::IdeasLaws | PoliticsTab::Autonomy => {
            let tabs = politics_tab_fallback(screen);
            let y = tabs.y + widgets::tabs::BAR_THICK;
            Rect {
                x: tabs.x + 17.0,
                y,
                w: (tabs.w - 17.0).max(0.0),
                h: (tabs.y + tabs.h - y).max(0.0),
            }
        }
    }
}

fn politics_grid_viewport(roots: &[Node], tab: PoliticsTab, fallback_viewport: Rect) -> Rect {
    find_rect(roots, tab.grid_name()).unwrap_or(fallback_viewport)
}

fn politics_list_window(
    viewport: Rect,
    row_count: usize,
    requested_scroll: f32,
    row_h: f32,
) -> PoliticsListWindow {
    let row_h = row_h.max(1.0);
    let content_h = row_count as f32 * row_h;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport.h, content_h);
    let scroll_y = requested_scroll.clamp(0.0, max_scroll);
    let first = (scroll_y / row_h).floor() as usize;
    let visible = (viewport.h / row_h).ceil().max(1.0) as usize + 1;
    PoliticsListWindow {
        viewport,
        scroll_y,
        max_scroll,
        first,
        visible,
    }
}

fn visible_politics_row_digest(rows: &[DataRow], window: PoliticsListWindow) -> String {
    let visible: Vec<DataRow> = rows
        .iter()
        .skip(window.first)
        .take(window.visible)
        .cloned()
        .collect();
    row_digest(&visible)
}

fn max_politics_scroll_fallback(viewport: Rect, row_count: usize, row_h: f32) -> f32 {
    (row_count as f32 * row_h.max(1.0) - viewport.h).max(0.0)
}

fn politics_scrollbar_geometry(
    viewport: Rect,
    row_count: usize,
    row_h: f32,
    scroll_y: f32,
) -> Option<PoliticsScrollbarGeometry> {
    let content_h = row_count as f32 * row_h.max(1.0);
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
    Some(PoliticsScrollbarGeometry {
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

fn politics_scroll_from_thumb_y(geom: PoliticsScrollbarGeometry, thumb_y: f32) -> f32 {
    let travel = (geom.track.h - geom.thumb.h).max(0.0);
    if travel <= 0.0 || geom.max_scroll <= 0.0 {
        return 0.0;
    }
    let y = thumb_y.clamp(geom.track.y, geom.track.y + travel);
    ((y - geom.track.y) / travel) * geom.max_scroll
}

fn scroll_politics_offset(
    current: f32,
    viewport: Rect,
    row_count: usize,
    row_h: f32,
    dy: f32,
) -> f32 {
    if dy == 0.0 || !dy.is_finite() {
        return current;
    }
    let row_h = row_h.max(1.0);
    let rows = dy.abs().ceil().max(1.0);
    let delta = rows * row_h;
    let next = if dy < 0.0 {
        current + delta
    } else {
        (current - delta).max(0.0)
    };
    next.min(max_politics_scroll_fallback(viewport, row_count, row_h))
}

fn pick_politics_row_key(
    viewport: Rect,
    keys: &[Option<String>],
    row_h: f32,
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
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / row_h.max(1.0)).floor() as usize;
    keys.get(idx).and_then(|key| key.clone())
}

fn click_politics_tab(bar: &mut TabBar, panel: Rect, x: f32, y: f32) -> PoliticsTabClick {
    let rects = bar.layout(panel);
    if let Some(id) = bar.pick(&rects, x, y) {
        if bar.select(&id) {
            PoliticsTabClick::Changed
        } else {
            PoliticsTabClick::Unchanged
        }
    } else if rects.iter().any(|tr| tr.rect.contains(x, y)) {
        PoliticsTabClick::Inert
    } else {
        PoliticsTabClick::Miss
    }
}

fn politics_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
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

fn politics_government_row_key(ideology: &str) -> Option<String> {
    if ideology.is_empty() {
        None
    } else {
        Some(format!("politics:government:ideology:{ideology}"))
    }
}

fn politics_token_row_key(source_label: &str, token: &str) -> Option<String> {
    if source_label.is_empty() || token.is_empty() {
        None
    } else {
        Some(format!("politics:token:{source_label}:{token}"))
    }
}

fn politics_overlord_row_key(overlord: &str) -> Option<String> {
    if overlord.is_empty() {
        None
    } else {
        Some(format!("politics:autonomy:overlord:{overlord}"))
    }
}

fn politics_subject_row_key(subject: &str, overlord: &str) -> Option<String> {
    if subject.is_empty() || overlord.is_empty() {
        None
    } else {
        Some(format!(
            "politics:autonomy:subject:{subject}:overlord:{overlord}"
        ))
    }
}

fn politics_occupied_owner_row_key(owner: &str, controller: &str) -> Option<String> {
    if owner.is_empty() || controller.is_empty() {
        None
    } else {
        Some(format!(
            "politics:autonomy:occupied-owner:{owner}:controller:{controller}"
        ))
    }
}

fn advisor_role_row_key(slot: &str) -> Option<String> {
    if slot.is_empty() {
        None
    } else {
        Some(format!("politics:advisor-role:slot:{slot}"))
    }
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
        let mut row = match politics_government_row_key(k) {
            Some(key) => DataRow::new().with_key(key),
            None => DataRow::new(),
        };
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
    suppress_duplicate_row_keys(&mut rows);
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

fn build_politics_tab_rows(
    ctx: &BindContext,
    tag: &str,
    tab: Option<PoliticsTab>,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    match tab {
        Some(PoliticsTab::Government) => build_party_rows(ctx, tag, fidelity),
        Some(PoliticsTab::NationalSpirits) => build_token_rows(
            ctx,
            tag,
            &[("National Spirit", "politics.ideas_other")],
            fidelity,
        ),
        Some(PoliticsTab::IdeasLaws) => build_token_rows(
            ctx,
            tag,
            &[
                ("Law", "politics.laws"),
                ("Advisor", "politics.appointed_advisors"),
                ("Idea", "politics.ideas"),
            ],
            fidelity,
        ),
        Some(PoliticsTab::Autonomy) => build_autonomy_tab_rows(ctx, tag, fidelity),
        None => {
            SectionState::Absent.record(fidelity);
            (Vec::new(), SectionState::Absent)
        }
    }
}

fn build_token_rows(
    ctx: &BindContext,
    tag: &str,
    sources: &[(&str, &str)],
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let mut rows = Vec::new();
    let mut saw_present = false;
    let mut saw_unusable = false;
    for (label, path) in sources {
        match ctx.country_field(tag, path) {
            Captured::Measured(v) => {
                saw_present = true;
                let entries = token_entries(v);
                for token in entries {
                    push_category_tab_row(
                        fidelity,
                        &mut rows,
                        &format!("{label}: {token}"),
                        politics_token_row_key(label, &token),
                    );
                }
            }
            Captured::Null => saw_unusable = true,
            Captured::Absent => {}
        }
    }
    let state = if saw_present {
        SectionState::Present
    } else if saw_unusable {
        SectionState::Null
    } else {
        SectionState::Absent
    };
    state.record(fidelity);
    suppress_duplicate_row_keys(&mut rows);
    (rows, state)
}

fn token_entries(v: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    match v {
        serde_json::Value::Array(arr) => {
            for item in arr {
                let token = fmt_token(item);
                if token != DASH {
                    out.push(token);
                }
            }
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(value) = map.get(key) {
                    let token = fmt_token(value);
                    if token != DASH {
                        out.push(token);
                    } else {
                        out.push(key.clone());
                    }
                }
            }
        }
        serde_json::Value::String(_) | serde_json::Value::Number(_) => out.push(fmt_token(v)),
        _ => {}
    }
    out
}

fn push_category_tab_row(
    fidelity: &mut FidelityReport,
    rows: &mut Vec<DataRow>,
    label: &str,
    key: Option<String>,
) {
    let mut row = match key {
        Some(key) => DataRow::new().with_key(key),
        None => DataRow::new(),
    };
    let label_value = serde_json::Value::String(label.to_string());
    row.bind(
        fidelity,
        "name",
        Captured::Measured(&label_value),
        Fidelity::Derived,
        fmt_token,
    );
    row.badge_unavailable(fidelity, "category_icon");
    rows.push(row);
}

fn build_autonomy_tab_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let mut rows = Vec::new();

    let overlord = ctx.country_field(tag, "diplomacy.overlord");
    let overlord = if overlord.is_measured() {
        overlord
    } else {
        ctx.country_field(tag, "politics.autonomy.overlord")
    };
    if let Captured::Measured(v) = overlord {
        let token = fmt_token(v);
        if token != DASH {
            push_category_tab_row(
                fidelity,
                &mut rows,
                &format!("Overlord: {token}"),
                politics_overlord_row_key(&token),
            );
        }
    }

    if let Some(countries) = ctx.countries() {
        let mut subjects: Vec<(&str, Option<String>)> = Vec::new();
        for (sub_tag, _) in countries {
            let over = ctx.country_field(sub_tag, "diplomacy.overlord");
            let over = if over.is_measured() {
                over
            } else {
                ctx.country_field(sub_tag, "politics.autonomy.overlord")
            };
            if let Captured::Measured(v) = over {
                if v.as_str() == Some(tag) {
                    let level = ctx.country_field(sub_tag, "diplomacy.autonomy_state");
                    let level = if level.is_measured() {
                        level
                    } else {
                        ctx.country_field(sub_tag, "politics.autonomy.level")
                    };
                    let level = match level {
                        Captured::Measured(v) => {
                            let token = fmt_token(v);
                            if token != DASH {
                                Some(token)
                            } else {
                                None
                            }
                        }
                        Captured::Absent | Captured::Null => None,
                    };
                    subjects.push((sub_tag.as_str(), level));
                }
            }
        }
        subjects.sort_by(|a, b| a.0.cmp(b.0));
        for (sub_tag, level) in subjects {
            let label = match level {
                Some(level) => format!("Subject {sub_tag}: {level}"),
                None => format!("Subject {sub_tag}"),
            };
            push_category_tab_row(
                fidelity,
                &mut rows,
                &label,
                politics_subject_row_key(sub_tag, tag),
            );
        }
    }

    if let Some(states) = ctx.states() {
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
        for owner in owners {
            push_category_tab_row(
                fidelity,
                &mut rows,
                &format!("Occupied {owner}"),
                politics_occupied_owner_row_key(&owner, tag),
            );
        }
    }

    suppress_duplicate_row_keys(&mut rows);
    let state = if rows.is_empty() {
        SectionState::Absent
    } else {
        SectionState::Present
    };
    state.record(fidelity);
    (rows, state)
}

fn politics_tab_keys_from_rebind(
    ctx: &RebindContext,
    tag: &str,
    tab: PoliticsTab,
) -> Vec<Option<String>> {
    match tab {
        PoliticsTab::Government => politics_party_keys_from_rebind(ctx, tag),
        PoliticsTab::NationalSpirits => politics_token_keys_from_rebind(
            ctx,
            tag,
            &[("National Spirit", "politics.ideas_other")],
        ),
        PoliticsTab::IdeasLaws => politics_token_keys_from_rebind(
            ctx,
            tag,
            &[
                ("Law", "politics.laws"),
                ("Advisor", "politics.appointed_advisors"),
                ("Idea", "politics.ideas"),
            ],
        ),
        PoliticsTab::Autonomy => politics_autonomy_keys_from_rebind(ctx, tag),
    }
}

fn politics_party_keys_from_rebind(ctx: &RebindContext, tag: &str) -> Vec<Option<String>> {
    let mut keys = match ctx.country_field(tag, "politics.party_popularities") {
        Captured::Measured(v) => match v.as_object() {
            Some(obj) => {
                let mut parts: Vec<&String> = obj.keys().collect();
                parts.sort();
                parts
                    .into_iter()
                    .map(|k| politics_government_row_key(k))
                    .collect()
            }
            None => Vec::new(),
        },
        Captured::Null | Captured::Absent => Vec::new(),
    };
    suppress_duplicate_keys(&mut keys);
    keys
}

fn politics_token_keys_from_rebind(
    ctx: &RebindContext,
    tag: &str,
    sources: &[(&str, &str)],
) -> Vec<Option<String>> {
    let mut keys = Vec::new();
    for (label, path) in sources {
        if let Captured::Measured(v) = ctx.country_field(tag, path) {
            for token in token_entries(v) {
                keys.push(politics_token_row_key(label, &token));
            }
        }
    }
    suppress_duplicate_keys(&mut keys);
    keys
}

fn politics_autonomy_keys_from_rebind(ctx: &RebindContext, tag: &str) -> Vec<Option<String>> {
    let mut keys = Vec::new();

    let overlord = ctx.country_field(tag, "diplomacy.overlord");
    let overlord = if overlord.is_measured() {
        overlord
    } else {
        ctx.country_field(tag, "politics.autonomy.overlord")
    };
    if let Captured::Measured(v) = overlord {
        if let Some(token) = row_key_token(v) {
            keys.push(politics_overlord_row_key(&token));
        }
    }

    if let Some(countries) = ctx.countries() {
        let mut subjects: Vec<&str> = Vec::new();
        for (sub_tag, _) in countries {
            if sub_tag == tag {
                continue;
            }
            let over = ctx.country_field(sub_tag, "diplomacy.overlord");
            let over = if over.is_measured() {
                over
            } else {
                ctx.country_field(sub_tag, "politics.autonomy.overlord")
            };
            if let Captured::Measured(v) = over {
                if v.as_str() == Some(tag) {
                    subjects.push(sub_tag.as_str());
                }
            }
        }
        subjects.sort();
        for sub_tag in subjects {
            keys.push(politics_subject_row_key(sub_tag, tag));
        }
    }

    if let Some(states) = ctx.states() {
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
        for owner in owners {
            keys.push(politics_occupied_owner_row_key(&owner, tag));
        }
    }

    suppress_duplicate_keys(&mut keys);
    keys
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
        push_active_range_name(&pb, &mut fidelity, &mut summary);

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

/// Bind the current named BoP range from static `common/bop` thresholds. Missing
/// or unmatched DEF data badges unavailable instead of guessing a range.
fn push_active_range_name(
    pb: &Option<serde_json::Value>,
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
) {
    let atom = active_range_name_atom(pb, bop_catalog());
    report.record(&atom);
    summary.push(Cell {
        element: "active_range_name".to_string(),
        atom,
    });
}

fn active_range_name_atom(pb: &Option<serde_json::Value>, cat: &BopDefCatalog) -> DisplayAtom {
    let Some(track) = pb.as_ref() else {
        return DisplayAtom::unavailable_because(UnavailableReason::Absent);
    };
    let template = match track.get("template").and_then(serde_json::Value::as_str) {
        Some(s) if !s.is_empty() => s,
        _ => return DisplayAtom::unavailable_because(UnavailableReason::Blocked),
    };
    let value = match track.get("value").and_then(serde_json::Value::as_f64) {
        Some(v) if v.is_finite() => v,
        _ => return DisplayAtom::unavailable_because(UnavailableReason::Absent),
    };
    match cat.active_range_id(template, value) {
        Some(id) => DisplayAtom::text(id.to_string(), Fidelity::Derived),
        None => DisplayAtom::unavailable_because(UnavailableReason::Blocked),
    }
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
const COMMAND_CHARACTER_ROW_H: f32 = 70.0;

pub struct CommandCharacterView {
    selected_key: Option<String>,
    row_keys: Vec<Option<String>>,
    scroll_y: f32,
    scroll_drag: Option<CommandCharacterScrollDrag>,
    screen: Rect,
}

impl CommandCharacterView {
    pub fn new() -> CommandCharacterView {
        CommandCharacterView {
            selected_key: None,
            row_keys: Vec::new(),
            scroll_y: 0.0,
            scroll_drag: None,
            screen: default_politics_screen(),
        }
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
    selected_key: Option<String>,
    list_window: PoliticsListWindow,
    /// the per-row trait grids (N3). Their plans feed the digest so a scrub is exact.
    trait_trees: Vec<TraitTree>,
}

impl Drawable for CommandCharacterDrawable {
    fn draw_digest(&self) -> String {
        let selected_state = match self.selected_key.as_deref() {
            None => "none",
            Some(key) if self.char_rows.iter().any(|r| r.key.as_deref() == Some(key)) => "selected",
            Some(_) => "missing_selected",
        };
        let mut tdig = String::new();
        for (i, tree) in self.trait_trees.iter().enumerate() {
            let g = tree.to_graph();
            let cam = Canvas::fit(g.bounds(), [0.0, 0.0, 400.0, 200.0], 12.0);
            let (plan, _stats) = node_graph::plan_graph(&cam, &g);
            tdig.push_str(&format!("t{i}={};", plan.digest()));
        }
        format!(
            "CMDCHAR sect={} selected={} selected_key={} selected_row={} scroll={:.0}/{:.0} visible=[{}] rows={} TRAITS[{}]",
            self.section.tag(),
            selected_state,
            self.selected_key.as_deref().unwrap_or("none"),
            selected_row_digest(&self.char_rows, self.selected_key.as_deref()),
            self.list_window.scroll_y,
            self.list_window.max_scroll,
            visible_politics_row_digest(&self.char_rows, self.list_window),
            row_digest(&self.char_rows),
            tdig
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows_selected_in_viewport(
            fd,
            &self.file,
            "box_grid",
            "command_character_view_item",
            &self.char_rows,
            self.list_window.viewport,
            self.list_window.scroll_y,
            self.selected_key.as_deref(),
        );
        if let Some(grid) = find_rect(&self.roots, "box_grid") {
            let white = mint_white(fd);
            let assets = widget_assets(&white);
            for (i, tree) in self.trait_trees.iter().enumerate() {
                let g = tree.to_graph();
                let row_y = grid.y + i as f32 * COMMAND_CHARACTER_ROW_H - self.list_window.scroll_y;
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
    fn rebind(&mut self, ctx: &RebindContext) {
        self.screen = Rect {
            x: 0.0,
            y: 0.0,
            w: ctx.screen.0 as f32,
            h: ctx.screen.1 as f32,
        };
        self.row_keys = command_character_keys_from_rebind(ctx, ctx.country);
        let viewport = command_character_grid_fallback(self.screen);
        self.scroll_y = self.scroll_y.min(max_politics_scroll_fallback(
            viewport,
            self.row_keys.len(),
            COMMAND_CHARACTER_ROW_H,
        ));
        if self.scroll_drag.is_some()
            && politics_scrollbar_geometry(
                viewport,
                self.row_keys.len(),
                COMMAND_CHARACTER_ROW_H,
                self.scroll_y,
            )
            .is_none()
        {
            self.scroll_drag = None;
        }
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => {
                let viewport = command_character_grid_fallback(self.screen);
                let scroll_y = self.scroll_y.min(max_politics_scroll_fallback(
                    viewport,
                    self.row_keys.len(),
                    COMMAND_CHARACTER_ROW_H,
                ));
                if let Some(geom) = politics_scrollbar_geometry(
                    viewport,
                    self.row_keys.len(),
                    COMMAND_CHARACTER_ROW_H,
                    scroll_y,
                ) {
                    if geom.thumb.contains(x, y) {
                        self.scroll_drag = Some(CommandCharacterScrollDrag {
                            pointer_offset_y: y - geom.thumb.y,
                        });
                        return politics_input_consumed(false);
                    }
                    if geom.track.contains(x, y) {
                        return politics_input_consumed(false);
                    }
                }
                if let Some(key) = pick_politics_row_key(
                    viewport,
                    &self.row_keys,
                    COMMAND_CHARACTER_ROW_H,
                    scroll_y,
                    x,
                    y,
                ) {
                    let changed = self.selected_key.as_deref() != Some(key.as_str());
                    self.selected_key = Some(key);
                    politics_input_consumed(changed)
                } else {
                    InputOutcome::ignored()
                }
            }
            ViewInput::Wheel { x, y, dy } => {
                let viewport = command_character_grid_fallback(self.screen);
                if x < viewport.x
                    || x > viewport.x + viewport.w
                    || y < viewport.y
                    || y > viewport.y + viewport.h
                {
                    return InputOutcome::ignored();
                }
                let current = self.scroll_y;
                let next = scroll_politics_offset(
                    current,
                    viewport,
                    self.row_keys.len(),
                    COMMAND_CHARACTER_ROW_H,
                    dy,
                );
                self.scroll_y = next;
                politics_input_consumed(next != current)
            }
            ViewInput::PointerMove { x: _, y, drag } if drag => {
                let drag = match self.scroll_drag {
                    Some(drag) => drag,
                    None => return InputOutcome::ignored(),
                };
                let viewport = command_character_grid_fallback(self.screen);
                let geom = match politics_scrollbar_geometry(
                    viewport,
                    self.row_keys.len(),
                    COMMAND_CHARACTER_ROW_H,
                    self.scroll_y,
                ) {
                    Some(geom) => geom,
                    None => {
                        self.scroll_drag = None;
                        return InputOutcome::ignored();
                    }
                };
                let current = self.scroll_y;
                let next = politics_scroll_from_thumb_y(geom, y - drag.pointer_offset_y);
                self.scroll_y = next;
                politics_input_consumed(next != current)
            }
            ViewInput::PointerUp {
                button: PointerButton::Left,
                ..
            } => {
                if self.scroll_drag.take().is_some() {
                    politics_input_consumed(false)
                } else {
                    InputOutcome::ignored()
                }
            }
            _ => InputOutcome::ignored(),
        }
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, COMMAND_CHARACTER_FILE);

        let (char_rows, section, trait_trees) = build_character_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, COMMAND_CHARACTER_ROOT, &[]);
        let viewport =
            command_character_grid_viewport(&roots, command_character_grid_fallback(ctx.screen));
        let drawable = CommandCharacterDrawable {
            roots: roots.clone(),
            file,
            char_rows,
            section,
            selected_key: self.selected_key.clone(),
            list_window: politics_list_window(
                viewport,
                trait_trees.len(),
                self.scroll_y,
                COMMAND_CHARACTER_ROW_H,
            ),
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

#[derive(Clone, Copy)]
struct CommandCharacterScrollDrag {
    pointer_offset_y: f32,
}

const COMMAND_CHARACTER_SOURCES: &[(&str, &str)] = &[
    ("army_leaders", "commanders.army_leaders"),
    ("navy_leaders", "commanders.navy_leaders"),
    ("generals", "commanders.generals"),
    ("field_marshals", "commanders.field_marshals"),
    ("admirals", "commanders.admirals"),
];

/// The character roster: `command_character_view_item` rows from the LIVE
/// `commanders.{army_leaders,navy_leaders}` (M). Per row: name/skill/traits/portrait/cost.
/// In the SAVE-only demo `commanders` is unfed → 0 rows, section badged absent (NOT a
/// fabricated empty present list). Returns rows + section + a parallel trait-tree per row.
fn build_character_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState, Vec<TraitTree>) {
    let mut leaders: Vec<(&'static str, &serde_json::Value)> = Vec::new();
    let mut section = SectionState::Absent;
    for (bucket, path) in COMMAND_CHARACTER_SOURCES {
        let (arr, st) = country_array(ctx, tag, path);
        if let Some(a) = arr {
            if section == SectionState::Absent {
                section = st;
            }
            leaders.extend(a.iter().map(|leader| (*bucket, leader)));
        }
    }
    let mut rows = Vec::new();
    let mut trees = Vec::new();
    for (bucket, l) in leaders.iter() {
        let mut row = match command_character_row_key(bucket, l) {
            Some(key) => DataRow::new().with_key(key),
            None => DataRow::new(),
        };
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
    suppress_duplicate_row_keys(&mut rows);
    if rows.is_empty() {
        section.record(fidelity);
    }
    (rows, section, trees)
}

fn command_character_keys_from_rebind(ctx: &RebindContext, tag: &str) -> Vec<Option<String>> {
    let mut keys = Vec::new();
    for (bucket, path) in COMMAND_CHARACTER_SOURCES {
        if let Captured::Measured(v) = ctx.country_field(tag, path) {
            if let Some(arr) = v.as_array() {
                keys.extend(
                    arr.iter()
                        .map(|leader| command_character_row_key(bucket, leader)),
                );
            }
        }
    }
    suppress_duplicate_keys(&mut keys);
    keys
}

fn command_character_row_key(bucket: &str, leader: &serde_json::Value) -> Option<String> {
    for field in ["char_id", "unit_leader_id", "id"] {
        if let Some(token) = leader.get(field).and_then(row_key_token) {
            return Some(format!("command_character:{bucket}:{field}:{token}"));
        }
    }
    None
}

fn command_character_grid_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 356.0 + 10.0,
        y: screen.y + 80.0 + 98.0 + 10.0,
        w: 490.0,
        h: (590.0_f32 - 98.0 - 10.0).max(0.0),
    }
}

fn command_character_grid_viewport(roots: &[Node], fallback_viewport: Rect) -> Rect {
    match find_rect(roots, "box_grid") {
        Some(rect) if rect.w >= COMMAND_CHARACTER_ROW_H && rect.h >= COMMAND_CHARACTER_ROW_H => {
            rect
        }
        _ => fallback_viewport,
    }
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

pub struct AdvisorRoleView {
    disabled_select_targets: RefCell<Vec<Rect>>,
    disabled_select_armed: bool,
}

impl AdvisorRoleView {
    pub fn new() -> AdvisorRoleView {
        AdvisorRoleView {
            disabled_select_targets: RefCell::new(Vec::new()),
            disabled_select_armed: false,
        }
    }

    fn disabled_select_hit(&self, x: f32, y: f32) -> bool {
        self.disabled_select_targets
            .borrow()
            .iter()
            .any(|rect| rect.contains(x, y))
    }

    #[cfg(test)]
    fn debug_disabled_select_targets(&self) -> Vec<Rect> {
        self.disabled_select_targets.borrow().clone()
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
    fn rebind(&mut self, _ctx: &RebindContext) {
        self.disabled_select_armed = false;
        self.disabled_select_targets.borrow_mut().clear();
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } if self.disabled_select_hit(x, y) => {
                self.disabled_select_armed = true;
                politics_input_consumed(false)
            }
            ViewInput::PointerMove { drag: true, .. } if self.disabled_select_armed => {
                politics_input_consumed(false)
            }
            ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            } if self.disabled_select_armed || self.disabled_select_hit(x, y) => {
                self.disabled_select_armed = false;
                politics_input_consumed(false)
            }
            _ => InputOutcome::ignored(),
        }
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
        let disabled_select_targets = instantiated_row_child_rects(
            ctx,
            &roots,
            &file,
            "roles_grid",
            "advisor_role_entry",
            role_rows.len(),
            0.0,
            "select_button",
        );
        *self.disabled_select_targets.borrow_mut() = disabled_select_targets;
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
        let mut row = match adv
            .get("slot")
            .and_then(row_key_token)
            .and_then(|slot| advisor_role_row_key(&slot))
        {
            Some(key) => DataRow::new().with_key(key),
            None => DataRow::new(),
        };
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
    suppress_duplicate_row_keys(&mut rows);
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

    fn stage_with_gui_file(name: &str, gui: GuiFile) -> Stage {
        let mut stage = stage_with(&[]);
        stage.gui_files.insert(name.to_string(), gui);
        stage
    }

    fn advisor_role_test_gui(include_select_button: bool) -> GuiFile {
        let mut template_children = vec![serde_json::json!({
            "type": "iconType",
            "name": "idea_icon",
            "position": {"x": 150, "y": 5},
            "size": {"width": 30, "height": 30}
        })];
        if include_select_button {
            template_children.push(serde_json::json!({
                "type": "buttonType",
                "name": "select_button",
                "position": {"x": 20, "y": 5},
                "size": {"width": 120, "height": 30}
            }));
        }
        let gui = serde_json::json!({
            "view": "advisor_role_selection_test",
            "windows": [
                {
                    "type": "containerWindowType",
                    "name": "advisorroleselectionview",
                    "position": {"x": 100, "y": 100},
                    "size": {"width": 500, "height": 360},
                    "children": [
                        {
                            "type": "instantTextBoxType",
                            "name": "name",
                            "position": {"x": 10, "y": 10},
                            "size": {"width": 140, "height": 20}
                        },
                        {
                            "type": "instantTextBoxType",
                            "name": "skill",
                            "position": {"x": 160, "y": 10},
                            "size": {"width": 50, "height": 20}
                        },
                        {
                            "type": "gridBoxType",
                            "name": "roles_grid",
                            "position": {"x": 10, "y": 60},
                            "size": {"width": 300, "height": 120}
                        }
                    ]
                },
                {
                    "type": "containerWindowType",
                    "name": "advisor_role_entry",
                    "size": {"width": 260, "height": 40},
                    "children": template_children
                }
            ]
        });
        GuiFile::parse(&gui.to_string()).expect("advisor-role test gui parses")
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

    fn rebind_politics(v: &mut PoliticsView, snap: &snapshot::Snapshot, refit: bool) {
        v.rebind(&RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit,
        });
    }

    fn rebind_command_character(
        v: &mut CommandCharacterView,
        snap: &snapshot::Snapshot,
        refit: bool,
    ) {
        v.rebind(&RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit,
        });
    }

    fn rebind_advisor_role(v: &mut AdvisorRoleView, snap: &snapshot::Snapshot, refit: bool) {
        v.rebind(&RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit,
        });
    }

    fn politics_tab_center(bar: &TabBar, panel: Rect, id: &str) -> (f32, f32) {
        let rect = bar
            .layout(panel)
            .into_iter()
            .find(|tr| tr.id == id)
            .unwrap_or_else(|| panic!("missing politics tab rect {id}"))
            .rect;
        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
    }

    fn rect_center(rect: Rect) -> (f32, f32) {
        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
    }

    fn click_politics_tab_center(v: &mut PoliticsView, id: &str) -> InputOutcome {
        let panel = politics_tab_fallback(v.screen);
        let (x, y) = politics_tab_center(&v.tabs, panel, id);
        v.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        })
    }

    fn active_politics_tab(v: &PoliticsView) -> PoliticsTab {
        selected_politics_tab(&v.tabs).expect("politics view has active tab")
    }

    fn click_politics_row(view: &mut PoliticsView, row: usize) -> InputOutcome {
        let tab = active_politics_tab(view);
        let viewport = politics_grid_fallback(view.screen, tab);
        view.on_input(&ViewInput::PointerDown {
            x: viewport.x + 4.0,
            y: viewport.y + tab.row_h() * row as f32 + 4.0,
            button: PointerButton::Left,
        })
    }

    fn click_command_character_row(view: &mut CommandCharacterView, row: usize) -> InputOutcome {
        let viewport = command_character_grid_fallback(view.screen);
        view.on_input(&ViewInput::PointerDown {
            x: viewport.x + 4.0,
            y: viewport.y + COMMAND_CHARACTER_ROW_H * row as f32 + 4.0,
            button: PointerButton::Left,
        })
    }

    fn wheel_command_character(view: &mut CommandCharacterView, dy: f32) -> InputOutcome {
        let viewport = command_character_grid_fallback(view.screen);
        view.on_input(&ViewInput::Wheel {
            x: viewport.x + 8.0,
            y: viewport.y + COMMAND_CHARACTER_ROW_H * 0.5,
            dy,
        })
    }

    fn wheel_politics(view: &mut PoliticsView, dy: f32) -> InputOutcome {
        let tab = active_politics_tab(view);
        let viewport = politics_grid_fallback(view.screen, tab);
        view.on_input(&ViewInput::Wheel {
            x: viewport.x + 8.0,
            y: viewport.y + tab.row_h() * 0.5,
            dy,
        })
    }

    fn command_character_scrollbar(v: &CommandCharacterView) -> PoliticsScrollbarGeometry {
        let viewport = command_character_grid_fallback(v.screen);
        politics_scrollbar_geometry(
            viewport,
            v.row_keys.len(),
            COMMAND_CHARACTER_ROW_H,
            v.scroll_y,
        )
        .expect("command-character list has scrollbar")
    }

    fn pointer_down_command_character_thumb(v: &mut CommandCharacterView) -> InputOutcome {
        let geom = command_character_scrollbar(v);
        v.on_input(&ViewInput::PointerDown {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.thumb.y + geom.thumb.h * 0.5,
            button: PointerButton::Left,
        })
    }

    fn politics_scrollbar(v: &PoliticsView) -> PoliticsScrollbarGeometry {
        let tab = active_politics_tab(v);
        let viewport = politics_grid_fallback(v.screen, tab);
        let state = v.tab_states.get(tab);
        politics_scrollbar_geometry(viewport, state.row_keys.len(), tab.row_h(), state.scroll_y)
            .expect("active politics list has scrollbar")
    }

    fn pointer_down_politics_thumb(v: &mut PoliticsView) -> InputOutcome {
        let geom = politics_scrollbar(v);
        v.on_input(&ViewInput::PointerDown {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.thumb.y + geom.thumb.h * 0.5,
            button: PointerButton::Left,
        })
    }

    fn politics_scroll_pair(d: &str) -> (i32, i32) {
        let rest = d
            .split(" scroll=")
            .nth(1)
            .expect("Politics digest has scroll marker");
        let pair = rest
            .split(" visible=")
            .next()
            .expect("Politics digest has visible marker");
        let (cur, max) = pair.split_once('/').expect("politics scroll pair");
        (
            cur.parse().expect("scroll current is int"),
            max.parse().expect("scroll max is int"),
        )
    }

    fn visible_politics_chunk(d: &str) -> &str {
        d.split(" visible=[")
            .nth(1)
            .and_then(|rest| rest.split("] rows=").next())
            .expect("Politics digest has visible row chunk")
    }

    fn command_character_scroll_pair(d: &str) -> (i32, i32) {
        let rest = d
            .split(" scroll=")
            .nth(1)
            .expect("CommandCharacter digest has scroll marker");
        let pair = rest
            .split(" visible=")
            .next()
            .expect("CommandCharacter digest has visible marker");
        let (cur, max) = pair.split_once('/').expect("command-character scroll pair");
        (
            cur.parse().expect("scroll current is int"),
            max.parse().expect("scroll max is int"),
        )
    }

    fn visible_command_character_chunk(d: &str) -> &str {
        d.split(" visible=[")
            .nth(1)
            .and_then(|rest| rest.split("] rows=").next())
            .expect("CommandCharacter digest has visible row chunk")
    }

    fn many_politics_snap(count: usize, prefix: &str) -> snapshot::Snapshot {
        let spirits: Vec<_> = (1..=count)
            .map(|idx| format!("{prefix}_spirit_{idx:02}"))
            .collect();
        let laws: Vec<_> = (1..=count)
            .map(|idx| format!("{prefix}_law_{idx:02}"))
            .collect();
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": {
                "GER": {
                    "politics": {
                        "ruling_party": "fascism",
                        "party_popularities": {"fascism": 1.0},
                        "ideas_other": spirits,
                        "laws": laws
                    },
                    "focus": {"current": "GER_rhineland"}
                }
            }
        }))
        .unwrap()
    }

    fn duplicate_spirits_snap() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": {
                "GER": {
                    "politics": {
                        "ruling_party": "fascism",
                        "party_popularities": {"fascism": 1.0},
                        "ideas_other": ["repeat_spirit", "repeat_spirit", "unique_spirit"]
                    }
                }
            }
        }))
        .unwrap()
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

    fn command_character_snap(prefix: &str) -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": {
                "GER": {
                    "commanders": {
                        "army_leaders": [
                            {"char_id": 11, "unit_leader_id": 101, "name": format!("{prefix}_Guderian"), "skill": 5, "traits": ["panzer_leader"]},
                            {"char_id": 12, "unit_leader_id": 102, "name": format!("{prefix}_Rommel"), "skill": 6, "traits": ["trickster"]}
                        ],
                        "navy_leaders": [
                            {"unit_leader_id": 201, "name": format!("{prefix}_Doenitz"), "skill": 4, "traits": ["seawolf"]}
                        ]
                    }
                }
            }
        }))
        .unwrap()
    }

    fn command_character_duplicate_and_unkeyed_snap() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": {
                "GER": {
                    "commanders": {
                        "army_leaders": [
                            {"char_id": 11, "name": "Duplicate One", "skill": 5, "traits": ["panzer_leader"]},
                            {"char_id": 11, "name": "Duplicate Two", "skill": 6, "traits": ["trickster"]},
                            {"name": "No Id", "skill": 3, "traits": ["defensive_doctrine"]},
                            {"unit_leader_id": 104, "name": "Unique", "skill": 4, "traits": ["organizer"]}
                        ]
                    }
                }
            }
        }))
        .unwrap()
    }

    fn command_character_missing_selected_snap() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1943.1.1.12", "hour_index": 400,
            "countries": {
                "GER": {
                    "commanders": {
                        "army_leaders": [
                            {"char_id": 21, "unit_leader_id": 301, "name": "B_Only", "skill": 3, "traits": ["cautious"]}
                        ]
                    }
                }
            }
        }))
        .unwrap()
    }

    fn many_command_character_snap(count: usize, prefix: &str) -> snapshot::Snapshot {
        let leaders: Vec<_> = (1..=count)
            .map(|idx| {
                serde_json::json!({
                    "char_id": idx,
                    "unit_leader_id": 1000 + idx,
                    "name": format!("{prefix}_Leader_{idx:02}"),
                    "skill": (idx % 6) + 1,
                    "traits": ["organizer"]
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "date": "1943.1.1.12", "hour_index": 400,
            "countries": {
                "GER": {
                    "commanders": {
                        "army_leaders": leaders
                    }
                }
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
    fn advisor_role_select_button_is_replay_disabled_noop() {
        let stage = stage_with_gui_file(ADVISOR_ROLE_FILE, advisor_role_test_gui(true));
        let snap = snap_b();
        let mut view = AdvisorRoleView::new();
        rebind_advisor_role(&mut view, &snap, true);

        let before_bound = view.bind(&ctx(&stage, &snap));
        let before = bound_digest(&before_bound);
        assert!(
            before.contains("key=politics:advisor-role:slot:political_advisor;"),
            "advisor-role rows must carry stable slot keys, got:\n{before}"
        );
        assert!(
            before.contains("key=politics:advisor-role:slot:high_command;"),
            "advisor-role rows must key the second slot too, got:\n{before}"
        );
        assert!(
            find_rect(&before_bound.hit.roots, "roles_grid").is_some(),
            "test must bind the real roles_grid geometry"
        );
        let targets = view.debug_disabled_select_targets();
        assert_eq!(
            targets.len(),
            2,
            "each real advisor-role select_button row leaf should be a disabled target"
        );
        let (x, y) = rect_center(targets[0]);

        let down = view.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "advisor-role select_button pointer-down must be a replay-disabled no-op"
        );
        let up = view.on_input(&ViewInput::PointerUp {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            up.consumed && !up.needs_redraw && up.commands.is_empty(),
            "advisor-role select_button pointer-up must also be a replay-disabled no-op"
        );

        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "disabled advisor-role select_button must not change rows, keys, or digest"
        );
    }

    #[test]
    fn advisor_role_missing_select_button_geometry_fails_closed() {
        let stage = stage_with_gui_file(ADVISOR_ROLE_FILE, advisor_role_test_gui(false));
        let snap = snap_b();
        let mut view = AdvisorRoleView::new();
        rebind_advisor_role(&mut view, &snap, true);

        let before_bound = view.bind(&ctx(&stage, &snap));
        let before = bound_digest(&before_bound);
        assert!(
            view.debug_disabled_select_targets().is_empty(),
            "missing select_button leaf must not create synthetic disabled targets"
        );
        let grid = find_rect(&before_bound.hit.roots, "roles_grid")
            .expect("test gui provides roles_grid geometry");
        let out = view.on_input(&ViewInput::PointerDown {
            x: grid.x + 40.0,
            y: grid.y + 10.0,
            button: PointerButton::Left,
        });
        assert!(
            !out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "without real select_button geometry the view must fail closed and ignore the click"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "missing geometry no-op probe must not change the advisor-role digest"
        );
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
    fn politics_tab_click_switches_digest_and_visible_content() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &snap, true);
        let cx = ctx(&stage, &snap);

        let government = bound_digest(&v.bind(&cx));
        assert!(government.contains("tab=government"), "{government}");
        assert!(government.contains("name=communism(D)"), "{government}");
        assert!(
            !government.contains("National Spirit: spirit_of_genius"),
            "default government tab must not include national-spirit rows: {government}"
        );

        let out = click_politics_tab_center(&mut v, POLITICS_TAB_NATIONAL_SPIRITS);
        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert!(out.commands.is_empty());
        let spirits = bound_digest(&v.bind(&cx));
        assert!(spirits.contains("tab=national_spirits"), "{spirits}");
        assert!(
            spirits.contains("National Spirit: spirit_of_genius"),
            "national-spirit tab must show spirit rows: {spirits}"
        );
        assert!(
            !spirits.contains("name=communism(D)"),
            "national-spirit tab must not keep government party rows: {spirits}"
        );

        let out = click_politics_tab_center(&mut v, POLITICS_TAB_IDEAS_LAWS);
        assert!(out.consumed);
        assert!(out.needs_redraw);
        let ideas_laws = bound_digest(&v.bind(&cx));
        assert!(ideas_laws.contains("tab=ideas_laws"), "{ideas_laws}");
        assert!(
            ideas_laws.contains("Advisor: ROOT_high_command_idea"),
            "ideas/laws tab must show appointed advisor rows: {ideas_laws}"
        );
        assert!(
            !ideas_laws.contains("National Spirit: spirit_of_genius"),
            "ideas/laws tab must not keep national-spirit rows: {ideas_laws}"
        );

        let out = click_politics_tab_center(&mut v, POLITICS_TAB_AUTONOMY);
        assert!(out.consumed);
        assert!(out.needs_redraw);
        let autonomy = bound_digest(&v.bind(&cx));
        assert!(autonomy.contains("tab=autonomy"), "{autonomy}");
        assert!(
            autonomy.contains("Subject SLO: autonomy_puppet"),
            "autonomy tab must show subject rows: {autonomy}"
        );
        assert!(
            autonomy.contains("Occupied FRA"),
            "autonomy tab must show occupied-country rows: {autonomy}"
        );
    }

    #[test]
    fn politics_unavailable_tab_click_is_explicitly_inert() {
        let stage = stage_with(&[]);
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12", "hour_index": 100,
            "countries": {
                "GER": {
                    "politics": {
                        "ruling_party": "fascism",
                        "party_popularities": {"fascism": 1.0}
                    }
                }
            }
        }))
        .unwrap();
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &snap, true);

        assert_eq!(v.tabs.selected(), Some(POLITICS_TAB_GOVERNMENT));
        let out = click_politics_tab_center(&mut v, POLITICS_TAB_NATIONAL_SPIRITS);
        assert!(
            out.consumed,
            "unavailable tab cell click is consumed by the panel"
        );
        assert!(
            !out.needs_redraw,
            "unavailable tab click must not request redraw"
        );
        assert_eq!(v.tabs.selected(), Some(POLITICS_TAB_GOVERNMENT));

        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(d.contains("tab=government"), "{d}");
        assert!(
            !d.contains("tab=national_spirits"),
            "inert click must not select the unavailable tab: {d}"
        );
    }

    #[test]
    fn politics_selected_tab_persists_across_scrub_a_b_a() {
        let stage = stage_with(&[]);
        let (a, b) = (snap_a(), snap_b());
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &a, true);
        let out = click_politics_tab_center(&mut v, POLITICS_TAB_NATIONAL_SPIRITS);
        assert!(out.consumed && out.needs_redraw);

        let da1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        rebind_politics(&mut v, &b, false);
        let db = bound_digest(&v.bind(&ctx(&stage, &b)));
        rebind_politics(&mut v, &a, false);
        let da2 = bound_digest(&v.bind(&ctx(&stage, &a)));

        assert!(da1.contains("tab=national_spirits"), "{da1}");
        assert!(da1.contains("National Spirit: spirit_of_genius"), "{da1}");
        assert!(db.contains("tab=national_spirits"), "{db}");
        assert!(db.contains("National Spirit: war_economy"), "{db}");
        assert!(
            !da2.contains("war_economy"),
            "A after scrub must not carry B-only national-spirit rows: {da2}"
        );
        assert_eq!(
            da1, da2,
            "scrub A->B->A must preserve selected politics tab without carrying B rows"
        );
    }

    #[test]
    fn politics_government_row_click_selects_ideology_key() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &snap, true);

        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected=none"), "{before}");

        let out = click_politics_row(&mut v, 2);
        assert!(out.consumed && out.needs_redraw);
        assert!(out.commands.is_empty());

        let selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(selected.contains("selected=selected"), "{selected}");
        assert!(
            selected.contains("selected_key=politics:government:ideology:fascism"),
            "government selection must use ideology token identity, got:\n{selected}"
        );
        assert!(
            selected.contains(
                "selected_row=r0[key=politics:government:ideology:fascism;name=fascism(D);"
            ),
            "selected row digest must point at fascism row, got:\n{selected}"
        );
    }

    #[test]
    fn politics_autonomy_row_click_selects_relationship_key() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &snap, true);
        let tab = click_politics_tab_center(&mut v, POLITICS_TAB_AUTONOMY);
        assert!(tab.consumed && tab.needs_redraw);

        let out = click_politics_row(&mut v, 0);
        assert!(out.consumed && out.needs_redraw);
        let selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected_key=politics:autonomy:subject:SLO:overlord:GER"),
            "autonomy selection must use subject/overlord identity, got:\n{selected}"
        );
        assert!(
            selected.contains("selected_row=r0[key=politics:autonomy:subject:SLO:overlord:GER;"),
            "selected autonomy row digest must point at the subject row, got:\n{selected}"
        );
    }

    #[test]
    fn politics_ideas_laws_row_click_selects_law_key_and_scrolled_backing_row() {
        let stage = stage_with(&[]);
        let snap = many_politics_snap(40, "A");
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &snap, true);
        let tab = click_politics_tab_center(&mut v, POLITICS_TAB_IDEAS_LAWS);
        assert!(tab.consumed && tab.needs_redraw);

        let top_click = click_politics_row(&mut v, 0);
        assert!(top_click.consumed && top_click.needs_redraw);
        let top_selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            top_selected.contains("selected_key=politics:token:Law:A_law_01"),
            "ideas/laws row selection must use the exact law token, got:\n{top_selected}"
        );

        let wheel = wheel_politics(&mut v, -2.0);
        assert!(wheel.consumed && wheel.needs_redraw);
        let scrolled = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(politics_scroll_pair(&scrolled).0, 200);
        assert!(
            visible_politics_chunk(&scrolled).contains("key=politics:token:Law:A_law_03;"),
            "ideas/laws visible slice must advance to the third law row: {scrolled}"
        );

        let scrolled_click = click_politics_row(&mut v, 0);
        assert!(scrolled_click.consumed && scrolled_click.needs_redraw);
        let scrolled_selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            scrolled_selected.contains("selected_key=politics:token:Law:A_law_03"),
            "scrolled ideas/laws click must select the backing law row, got:\n{scrolled_selected}"
        );
    }

    #[test]
    fn politics_duplicate_token_rows_are_unkeyed_and_inert() {
        let stage = stage_with(&[]);
        let snap = duplicate_spirits_snap();
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &snap, true);
        let tab = click_politics_tab_center(&mut v, POLITICS_TAB_NATIONAL_SPIRITS);
        assert!(tab.consumed && tab.needs_redraw);

        let duplicate = click_politics_row(&mut v, 0);
        assert!(
            !duplicate.consumed && !duplicate.needs_redraw && duplicate.commands.is_empty(),
            "duplicate measured token keys must not be selected by index"
        );
        assert!(v
            .tab_states
            .get(PoliticsTab::NationalSpirits)
            .selected_key
            .is_none());

        let unique = click_politics_row(&mut v, 2);
        assert!(unique.consumed && unique.needs_redraw);
        let selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected_key=politics:token:National Spirit:unique_spirit"),
            "unique token row should remain selectable, got:\n{selected}"
        );
    }

    #[test]
    fn politics_wheel_scroll_clamps_and_scrolled_click_selects_backing_key() {
        let stage = stage_with(&[]);
        let snap = many_politics_snap(40, "A");
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &snap, true);
        click_politics_tab_center(&mut v, POLITICS_TAB_NATIONAL_SPIRITS);

        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(politics_scroll_pair(&before).0, 0);
        assert!(
            visible_politics_chunk(&before)
                .contains("key=politics:token:National Spirit:A_spirit_01;"),
            "initial visible politics rows must start at spirit 1: {before}"
        );

        let out = wheel_politics(&mut v, -3.0);
        assert!(out.consumed && out.needs_redraw);
        let after = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(politics_scroll_pair(&after).0, 300);
        assert!(
            visible_politics_chunk(&after)
                .contains("key=politics:token:National Spirit:A_spirit_04;"),
            "after three-row wheel scroll, visible rows must advance: {after}"
        );
        assert!(
            !visible_politics_chunk(&after)
                .contains("key=politics:token:National Spirit:A_spirit_01;"),
            "scrolled visible slice must not keep top politics rows: {after}"
        );

        let clicked = click_politics_row(&mut v, 0);
        assert!(clicked.consumed && clicked.needs_redraw);
        let selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected_key=politics:token:National Spirit:A_spirit_04"),
            "scrolled row click must select backing spirit 4, got:\n{selected}"
        );

        let out = wheel_politics(&mut v, -500.0);
        assert!(out.consumed);
        let clamped = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let (cur, max) = politics_scroll_pair(&clamped);
        assert_eq!(cur, max, "huge wheel-down must clamp to max: {clamped}");

        let viewport = politics_grid_fallback(v.screen, active_politics_tab(&v));
        let outside = v.on_input(&ViewInput::Wheel {
            x: viewport.x + viewport.w + 20.0,
            y: viewport.y + POLITICS_TAB_ROW_H,
            dy: -5.0,
        });
        assert!(
            !outside.consumed && !outside.needs_redraw && outside.commands.is_empty(),
            "wheel outside the active politics list must be inert"
        );
    }

    #[test]
    fn politics_scrollbar_thumb_drag_clamps_captures_outside_and_clears_on_up() {
        let stage = stage_with(&[]);
        let snap = many_politics_snap(40, "A");
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &snap, true);
        click_politics_tab_center(&mut v, POLITICS_TAB_NATIONAL_SPIRITS);

        let down = pointer_down_politics_thumb(&mut v);
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "thumb pointer-down should capture drag without changing scroll"
        );
        let geom = politics_scrollbar(&v);
        let drag = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 300.0,
            y: geom.track.y + geom.track.h + 500.0,
            drag: true,
        });
        assert!(
            drag.consumed && drag.needs_redraw,
            "captured thumb drag must continue outside the politics list"
        );
        let bottom = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let (cur, max) = politics_scroll_pair(&bottom);
        assert_eq!(cur, max, "dragging below the track should clamp to max");

        let reset = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 300.0,
            y: geom.track.y - 500.0,
            drag: true,
        });
        assert!(reset.consumed && reset.needs_redraw);
        let top = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(politics_scroll_pair(&top).0, 0);

        let up = v.on_input(&ViewInput::PointerUp {
            x: geom.track.x - 300.0,
            y: geom.track.y,
            button: PointerButton::Left,
        });
        assert!(
            up.consumed && !up.needs_redraw,
            "pointer-up should consume solely to clear politics scrollbar capture"
        );
        let ignored = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 300.0,
            y: geom.track.y + geom.track.h,
            drag: true,
        });
        assert!(
            !ignored.consumed && !ignored.needs_redraw,
            "pointer move after release must not keep stale politics scrollbar capture"
        );
    }

    #[test]
    fn politics_scrollbar_track_click_is_consumed_and_inert() {
        let stage = stage_with(&[]);
        let snap = many_politics_snap(40, "A");
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &snap, true);
        click_politics_tab_center(&mut v, POLITICS_TAB_NATIONAL_SPIRITS);
        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let geom = politics_scrollbar(&v);
        let track_y = (geom.thumb.y + geom.thumb.h + 10.0).min(geom.track.y + geom.track.h - 1.0);

        let out = v.on_input(&ViewInput::PointerDown {
            x: geom.track.x + geom.track.w * 0.5,
            y: track_y,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "scrollbar track click must be a consumed inert UI hit"
        );
        assert!(
            v.tab_states
                .get(PoliticsTab::NationalSpirits)
                .selected_key
                .is_none(),
            "track click must not select an underlying politics row"
        );
        let after = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(before, after, "track click must not alter politics digest");
    }

    #[test]
    fn politics_ideas_laws_scrollbar_thumb_drag_and_track_click_are_proven() {
        let stage = stage_with(&[]);
        let snap = many_politics_snap(40, "A");
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &snap, true);
        click_politics_tab_center(&mut v, POLITICS_TAB_IDEAS_LAWS);

        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(politics_scroll_pair(&before).0, 0);
        assert!(
            visible_politics_chunk(&before).contains("key=politics:token:Law:A_law_01;"),
            "ideas/laws visible rows must begin at the first law row: {before}"
        );

        let geom = politics_scrollbar(&v);
        let track_y = (geom.thumb.y + geom.thumb.h + 10.0).min(geom.track.y + geom.track.h - 1.0);
        let track = v.on_input(&ViewInput::PointerDown {
            x: geom.track.x + geom.track.w * 0.5,
            y: track_y,
            button: PointerButton::Left,
        });
        assert!(
            track.consumed && !track.needs_redraw && track.commands.is_empty(),
            "ideas/laws scrollbar track click must be a consumed inert UI hit"
        );
        assert!(
            v.tab_states
                .get(PoliticsTab::IdeasLaws)
                .selected_key
                .is_none(),
            "ideas/laws track click must not select an underlying row"
        );
        let after_track = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after_track,
            "track click must not alter ideas/laws digest"
        );

        let down = pointer_down_politics_thumb(&mut v);
        assert!(down.consumed && !down.needs_redraw && down.commands.is_empty());
        let drag = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 200.0,
            y: geom.track.y + geom.track.h + 400.0,
            drag: true,
        });
        assert!(
            drag.consumed && drag.needs_redraw,
            "captured ideas/laws thumb drag must continue outside the list"
        );
        let bottom = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let (cur, max) = politics_scroll_pair(&bottom);
        assert_eq!(
            cur, max,
            "ideas/laws thumb drag below track must clamp to max"
        );

        let up = v.on_input(&ViewInput::PointerUp {
            x: geom.track.x - 200.0,
            y: geom.track.y + geom.track.h,
            button: PointerButton::Left,
        });
        assert!(up.consumed && !up.needs_redraw);
    }

    #[test]
    fn politics_per_tab_scroll_state_is_isolated() {
        let stage = stage_with(&[]);
        let snap = many_politics_snap(40, "A");
        let mut v = PoliticsView::new();
        rebind_politics(&mut v, &snap, true);

        click_politics_tab_center(&mut v, POLITICS_TAB_NATIONAL_SPIRITS);
        wheel_politics(&mut v, -4.0);
        let spirits = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(politics_scroll_pair(&spirits).0, 400);

        click_politics_tab_center(&mut v, POLITICS_TAB_IDEAS_LAWS);
        let ideas_initial = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(
            politics_scroll_pair(&ideas_initial).0,
            0,
            "ideas/laws tab must not inherit national-spirit scroll"
        );
        assert!(
            visible_politics_chunk(&ideas_initial).contains("key=politics:token:Law:A_law_01;"),
            "ideas/laws visible rows must start from its own list: {ideas_initial}"
        );
        wheel_politics(&mut v, -2.0);
        let ideas_scrolled = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(politics_scroll_pair(&ideas_scrolled).0, 200);

        click_politics_tab_center(&mut v, POLITICS_TAB_NATIONAL_SPIRITS);
        let spirits_again = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(
            politics_scroll_pair(&spirits_again).0,
            400,
            "national-spirit scroll should survive switching away and back"
        );
    }

    #[test]
    fn politics_scroll_and_selection_scrub_a_b_a_without_carrying_b_rows() {
        let stage = stage_with(&[]);
        let a = many_politics_snap(40, "A");
        let b = many_politics_snap(45, "B");
        let mut v = PoliticsView::new();

        rebind_politics(&mut v, &a, true);
        click_politics_tab_center(&mut v, POLITICS_TAB_NATIONAL_SPIRITS);
        wheel_politics(&mut v, -5.0);
        click_politics_row(&mut v, 0);
        let a1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert!(
            a1.contains("selected_key=politics:token:National Spirit:A_spirit_06"),
            "{a1}"
        );
        assert!(
            visible_politics_chunk(&a1).contains("key=politics:token:National Spirit:A_spirit_06;"),
            "A visible chunk should include selected row after scroll: {a1}"
        );

        rebind_politics(&mut v, &b, false);
        let b_digest = bound_digest(&v.bind(&ctx(&stage, &b)));
        assert!(
            b_digest.contains("selected=missing_selected"),
            "B uses different measured spirit ids, so A's selection must fail closed: {b_digest}"
        );
        assert_eq!(
            politics_scroll_pair(&b_digest).0,
            politics_scroll_pair(&a1).0
        );
        assert!(
            visible_politics_chunk(&b_digest)
                .contains("key=politics:token:National Spirit:B_spirit_06;"),
            "B visible chunk must rebuild from B rows at retained scroll: {b_digest}"
        );
        assert!(
            !visible_politics_chunk(&b_digest)
                .contains("key=politics:token:National Spirit:A_spirit_06;"),
            "B visible chunk must not carry A rows: {b_digest}"
        );

        rebind_politics(&mut v, &a, false);
        let a2 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "politics scroll + selection must be byte-identical across A->B->A"
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
    fn command_character_row_click_selects_measured_character_key() {
        let stage = stage_with(&["command_character.json"]);
        let snap = command_character_snap("A");
        let mut v = CommandCharacterView::new();
        rebind_command_character(&mut v, &snap, true);

        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected=none"), "{before}");
        let out = click_command_character_row(&mut v, 1);
        assert!(out.consumed && out.needs_redraw && out.commands.is_empty());
        let selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(selected.contains("selected=selected"), "{selected}");
        assert!(
            selected.contains("selected_key=command_character:army_leaders:char_id:12"),
            "command-character selection must use measured char_id identity, got:\n{selected}"
        );
        assert!(
            selected.contains(
                "selected_row=r0[key=command_character:army_leaders:char_id:12;name=A_Rommel(D);"
            ),
            "selected row digest must point at Rommel, got:\n{selected}"
        );
    }

    #[test]
    fn command_character_duplicate_and_unkeyed_rows_are_inert() {
        let stage = stage_with(&["command_character.json"]);
        let snap = command_character_duplicate_and_unkeyed_snap();
        let mut v = CommandCharacterView::new();
        rebind_command_character(&mut v, &snap, true);

        let duplicate = click_command_character_row(&mut v, 0);
        assert!(
            !duplicate.consumed && !duplicate.needs_redraw && duplicate.commands.is_empty(),
            "duplicate measured commander ids must not be selected by index"
        );
        let unkeyed = click_command_character_row(&mut v, 2);
        assert!(
            !unkeyed.consumed && !unkeyed.needs_redraw && unkeyed.commands.is_empty(),
            "id-less commander rows must stay inert"
        );
        let unique = click_command_character_row(&mut v, 3);
        assert!(unique.consumed && unique.needs_redraw && unique.commands.is_empty());
        let selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected_key=command_character:army_leaders:unit_leader_id:104"),
            "unique measured commander id should remain selectable, got:\n{selected}"
        );
    }

    #[test]
    fn command_character_selected_row_scrub_a_b_a_fails_closed_when_missing() {
        let stage = stage_with(&["command_character.json"]);
        let a = command_character_snap("A");
        let b = command_character_missing_selected_snap();
        let mut v = CommandCharacterView::new();

        rebind_command_character(&mut v, &a, true);
        let out = click_command_character_row(&mut v, 1);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert!(
            a1.contains("selected_key=command_character:army_leaders:char_id:12"),
            "{a1}"
        );

        rebind_command_character(&mut v, &b, false);
        let missing = bound_digest(&v.bind(&ctx(&stage, &b)));
        assert!(
            missing.contains("selected=missing_selected"),
            "missing selected commander must be explicit, got:\n{missing}"
        );
        assert!(
            missing.contains("selected_key=command_character:army_leaders:char_id:12"),
            "missing selected commander must keep the measured selected key, got:\n{missing}"
        );
        assert!(
            !missing.contains("selected_row=r0[key=command_character:army_leaders:char_id:21;"),
            "missing selected commander must not fall back to another row, got:\n{missing}"
        );

        rebind_command_character(&mut v, &a, false);
        let a2 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "command-character selected row must be byte-identical across A->B->A"
        );
    }

    #[test]
    fn command_character_wheel_scroll_clamps_and_scrolled_click_selects_backing_key() {
        let stage = stage_with(&["command_character.json"]);
        let snap = many_command_character_snap(12, "A");
        let mut v = CommandCharacterView::new();
        rebind_command_character(&mut v, &snap, true);

        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(command_character_scroll_pair(&before).0, 0);
        assert!(
            visible_command_character_chunk(&before)
                .contains("key=command_character:army_leaders:char_id:1;"),
            "initial visible command-character rows must start at leader 1: {before}"
        );

        let out = wheel_command_character(&mut v, -3.0);
        assert!(out.consumed && out.needs_redraw && out.commands.is_empty());
        let after = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(command_character_scroll_pair(&after).0, 210);
        assert!(
            visible_command_character_chunk(&after)
                .contains("key=command_character:army_leaders:char_id:4;"),
            "after three-row wheel scroll, visible rows must advance: {after}"
        );
        assert!(
            !visible_command_character_chunk(&after)
                .contains("key=command_character:army_leaders:char_id:1;"),
            "scrolled visible slice must not keep top command-character rows: {after}"
        );

        let clicked = click_command_character_row(&mut v, 0);
        assert!(clicked.consumed && clicked.needs_redraw && clicked.commands.is_empty());
        let selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected_key=command_character:army_leaders:char_id:4"),
            "scrolled command-character click must select backing leader 4, got:\n{selected}"
        );

        for _ in 0..20 {
            wheel_command_character(&mut v, -1.0);
        }
        let clamped = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let (cur, max) = command_character_scroll_pair(&clamped);
        assert_eq!(
            cur, max,
            "command-character wheel must clamp to max: {clamped}"
        );
    }

    #[test]
    fn command_character_scrollbar_thumb_drag_clamps_captures_outside_and_clears_on_up() {
        let stage = stage_with(&["command_character.json"]);
        let snap = many_command_character_snap(12, "A");
        let mut v = CommandCharacterView::new();
        rebind_command_character(&mut v, &snap, true);

        let down = pointer_down_command_character_thumb(&mut v);
        assert!(down.consumed && !down.needs_redraw && down.commands.is_empty());
        assert!(
            v.scroll_drag.is_some(),
            "thumb pointer-down should capture command-character drag"
        );

        let geom = command_character_scrollbar(&v);
        let moved = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x + geom.track.w + 40.0,
            y: geom.track.y + geom.track.h + 120.0,
            drag: true,
        });
        assert!(
            moved.consumed && moved.needs_redraw && moved.commands.is_empty(),
            "captured command-character thumb drag should continue outside the list"
        );
        let bottom = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let (cur, max) = command_character_scroll_pair(&bottom);
        assert_eq!(
            cur, max,
            "thumb drag below track must clamp to max: {bottom}"
        );

        let up = v.on_input(&ViewInput::PointerUp {
            x: geom.track.x,
            y: geom.track.y,
            button: PointerButton::Left,
        });
        assert!(up.consumed && !up.needs_redraw && up.commands.is_empty());
        assert!(
            v.scroll_drag.is_none(),
            "pointer-up must clear drag capture"
        );

        let after_up = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x,
            y: geom.track.y,
            drag: true,
        });
        assert!(
            !after_up.consumed && !after_up.needs_redraw,
            "pointer move after thumb release must not keep stale capture"
        );
    }

    #[test]
    fn command_character_scrollbar_track_click_is_consumed_and_inert() {
        let stage = stage_with(&["command_character.json"]);
        let snap = many_command_character_snap(12, "A");
        let mut v = CommandCharacterView::new();
        rebind_command_character(&mut v, &snap, true);
        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let geom = command_character_scrollbar(&v);

        let out = v.on_input(&ViewInput::PointerDown {
            x: geom.track.x + geom.track.w * 0.5,
            y: (geom.track.y + geom.thumb.y) * 0.5,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "command-character scrollbar track click should be a consumed inert UI hit"
        );
        let after = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "track click must not select an underlying command-character row"
        );
    }

    #[test]
    fn command_character_scroll_and_selection_scrub_a_b_a_without_carrying_b_rows() {
        let stage = stage_with(&["command_character.json"]);
        let a = many_command_character_snap(12, "A");
        let b = many_command_character_snap(12, "B");
        let mut v = CommandCharacterView::new();

        rebind_command_character(&mut v, &a, true);
        wheel_command_character(&mut v, -3.0);
        click_command_character_row(&mut v, 0);
        let a1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert!(
            a1.contains("selected_key=command_character:army_leaders:char_id:4"),
            "{a1}"
        );

        rebind_command_character(&mut v, &b, false);
        let b_digest = bound_digest(&v.bind(&ctx(&stage, &b)));
        assert_eq!(
            command_character_scroll_pair(&b_digest).0,
            command_character_scroll_pair(&a1).0
        );
        assert!(
            visible_command_character_chunk(&b_digest).contains("name=B_Leader_04(D);"),
            "B visible chunk must rebuild from B rows at retained scroll: {b_digest}"
        );
        assert!(
            !visible_command_character_chunk(&b_digest).contains("name=A_Leader_04(D);"),
            "B visible chunk must not carry A rows: {b_digest}"
        );

        rebind_command_character(&mut v, &a, false);
        let a2 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "command-character scroll + selection must be byte-identical across A->B->A"
        );
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

    #[test]
    fn power_balance_catalog_derives_active_range_name() {
        let cat = BopDefCatalog::from_value(&serde_json::json!({
            "catalogs": {
                "power_balances": {
                    "TEST_balance": {
                        "neutral_ranges": [
                            {"id": "balanced_range", "min": "-0.1", "max": "0.1"}
                        ],
                        "sides": [
                            {"id": "left_side", "ranges": [
                                {"id": "left_range", "min": "-1", "max": "-0.1"}
                            ]},
                            {"id": "right_side", "ranges": [
                                {"id": "right_range", "min": "0.1", "max": "1"}
                            ]}
                        ]
                    }
                }
            }
        }))
        .expect("test catalog parses");
        let pb = Some(serde_json::json!({
            "template": "TEST_balance",
            "value": 0.5,
            "left_side": "left_side",
            "right_side": "right_side",
            "countries": ["GER"]
        }));

        let atom = active_range_name_atom(&pb, &cat);
        assert_eq!(atom.fidelity(), Fidelity::Derived);
        assert_eq!(atom.as_str(), "right_range");
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
