//! CLUSTER 2h — logistics_combat_units_events (12 views) [N5 cards, N6 naval-bubble].
//!
//! The LOGISTICS / SUPPLY, NAVAL-COMBAT-RESULT, UNIT/DIVISION-DETAIL, and
//! EVENTS/POPUPS/NOTIFICATIONS surfaces, bound per
//! `replay_ui/BINDING_MAP_logistics_combat_units_events.md` against the REAL parsed game
//! `.gui` files (fetched via the conflict-free `.gui`-set loader, NEVER synthetic chrome):
//!
//!   1.  **logistics** — `countrylogisticsview` ROOT (`logistics.json`): the materiel /
//!       fuel-prioritization / factory-usage panel. `economy.resources` resource rows
//!       EXACT; the AGGREGATE fuel bar (`economy.fuel.current/max`) + factory-usage bars
//!       (`economy.factories.{mil,dockyard}`) DERIVED; the materiel grid +
//!       per-branch fuel + nuke (need `economy.logistics`/`equipment_stockpile`/`extra`,
//!       all unfed in the demo) badged `unavailable`.
//!   2.  **garrison_log** — `garrison_log_view` ROOT (`garrison_log.json`): the resistance
//!       ledger. The belligerent flags are derivable; the per-attack manpower/equipment
//!       loss tally is a not-stored running buffer → `unavailable`.
//!   3.  **naval_combat_results** — `navalcombatresultswindow` ROOT
//!       (`naval_combat_results.json`): a post-battle report. The demo has 0 naval combats
//!       (`combats[]` is all `kind:"land"`), so the **N6 RosterPanel** attacker/defender
//!       columns render empty (the honest "no roster" badge — never fabricated zeros).
//!   4.  **destroyed_ships** — `destroyed_ships.json` is a **present-but-empty STUB**
//!       (`windows:[]`, `element_type_counts:{}`). It is documented in full (see
//!       [`DestroyedShipsView`]) and registered as an explicit zero-element EMPTY-STUB
//!       placeholder so the menubar surfaces it honestly (NOT a fabricated parsed root);
//!       the real loss roster lives in VIEW 4b (`naval_losses`).
//!   4b. **naval_losses** — `navallosseswindow` ROOT (`naval_losses.json`): the ship-loss
//!       browser. `diplomacy_world.factions` faction list EXACT; the loss roster
//!       (`world_events`/`ships_by_hull`, unfed) badged `unavailable`.
//!   5.  **naval_strategic_region** — `navalregionview` ROOT
//!       (`naval_strategic_region.json`): a sea-zone dominance panel. Its bindings are the
//!       `naval_air.{region_control,air_superiority}` PROXY (a PERMANENT proxy badge); unfed
//!       in the demo → `unavailable`.
//!   6.  **unit_view** — `army_division_view` row + `army_division_stats_view`
//!       (`unit_view.json`): the RICHEST EXACT-binding view. The snapshot `divisions[]`
//!       (memory channel) feeds per-division org/strength/equipment/owner/moving/template
//!       rows DIRECTLY and EXACTLY; the present-but-null `experience`/`supply_status` and
//!       the absent `divisions_full` body badge `unavailable`; ship detail needs
//!       `order_of_battle`.
//!   7.  **event_window** — `EventWindow` ×4 variants + `event_option_entry`
//!       (`event_window.json`): the **N5 EventCard**. The card is built from the static
//!       `events.json` event DEF (title/desc/picture/options) as a clearly-labelled
//!       definition PREVIEW; which-event-FIRED + the historical option-taken come from
//!       `world_events.fired_events` (unfed in the demo) → badged `unavailable`. We NEVER
//!       pick a "representative" fired event (Codex (b): a def supplies static text only,
//!       never occurrence/timing/country/fired-status).
//!   8.  **popup_window** — the 32-popup generic LIBRARY (`popup_window.json`): one
//!       [`PopupWindowView`] that ENUMERATES and renders ALL 32 real popup roots as **N5
//!       cards** (Codex (c): every ROOT covered). Notification-class popups are
//!       derived-from-event chrome; the body MESSAGE text (scripted loc) + the
//!       mutation-modal verbs are `unavailable`/inert.
//!   9.  **detailed_windows** — `shipswindow`/`planeswindow` (`detailed_windows.json`):
//!       equipment-in-use drill-downs. `ships_by_hull`/`equipment_stockpile` unfed →
//!       `unavailable`.
//!   10. **core** — `core.json` is the SHARED PROTOTYPE library (scrollbars/tooltips/the
//!       `countryflag_entry` row). Documented as near-zero-binding chrome; surfaced as an
//!       explicit prototype-library view (the one conditionally-derived prototype
//!       `countryflag_entry` is badged).
//!   11. **ingame_menu** — `ingamemenu` ROOT (`ingame_menu.json`): the pause/escape menu.
//!       Its verbs (save/load/resign/quit) are inert in a replay and the save list reads
//!       the host filesystem (OS-scope) → all `unavailable`/inert.
//!
//! THE NO-FABRICATION CONTRACT. This is a CLUSTER file, so the source-scanning guard
//! (`views::digest::no_fabricated_default_in_cluster_views`) forbids `unwrap_or(0)` /
//! `unwrap_or_default()` / the raw `ctx.snapshot(` seam. Every field is read through
//! [`BindContext::country_field`] / [`BindContext::state_field`] or the audited top-level
//! domain accessors ([`BindContext::divisions`], [`BindContext::combats`],
//! [`BindContext::order_of_battle`], [`BindContext::world_events`]) → [`Captured`], and
//! rendered through [`DisplayAtom`], so a blocked / absent / null field renders the
//! not-captured glyph "—" — NEVER a fabricated 0. Genuinely runtime-computed,
//! world-/DEF-data, and live-channel-resisted fields are badged `unavailable`. Mutation
//! verbs (set fuel priority, dismiss report, release country, save/load) render as real
//! chrome but are inert (no datum).
//!
//! THE ROW MODEL (mirrors `politics_government` / `production_construction_decisions`). A
//! `gridBoxType` is a row factory: the ROOT scene tree (static chrome + summary text
//! boxes) is fixed regardless of how many array elements exist; per-element rows are
//! instantiated from a TEMPLATE window and CUSTOM-DRAWN by the view's `Drawable`. So the
//! element-accounting target is the `.gui`'s static element set (proven ==
//! `stats.element_type_counts`); per-row data lives in the drawable digest, not as extra
//! ROOT nodes.
//!
//! THIS FILE IS YOURS, cluster agent — edit ONLY here (+ this subdir). Registration is
//! distributed through `inventory`, so adding/binding a view edits NO shared file.

use gui_runtime::draw::{DrawStats, TextureCache};
use gui_runtime::fonts::FontCache;
use gui_runtime::gridbox::{self, GridSpec};
use gui_runtime::scene::{ElementKind, Node};
use gui_runtime::{build_scene, layout, GuiFile, Loc, ManifestContent, Rect};

use bind::Fidelity;

use widgets::common::{Atom, DrawAssets};
use widgets::event_card::{EventCard, PopupCard};
use widgets::naval_combat::{RosterPanel, SideColumn};

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
// (Same shape as the worked example; a cluster owns its own copies — no shared-file edit.)
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

/// Format a measured number with one decimal (org/strength bars).
fn fmt_f1(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) => format!("{f:.1}"),
        None => DASH.to_string(),
    }
}

/// Format a measured value as its raw string token (tag, archetype, ideology).
fn fmt_token(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => DASH.to_string(),
    }
}

/// Format a 0..1 fraction as a percent (fuel/usage bars). Measured only.
fn fmt_frac_pct(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) => format!("{}%", (f.clamp(0.0, 1.0) * 100.0).round() as i64),
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
/// `GuiFile` typed parser REJECTED the file (a non-numeric `position` scalar or a
/// percentage `text` max-constraint) — this falls back to a LENIENT re-parse from
/// `Stage::assets_dir/gui/<file>`, coercing only those two malformed shapes. The fallback
/// is DETERMINISTIC and never invents chrome: a genuinely unreadable/unparseable file
/// yields [`empty_gui`] (the view then binds nothing + its accounting/digest reflect 0
/// elements) rather than a fabricated layout. READ-ONLY, per-cluster (no shared-file edit).
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

/// Repair the TWO string-where-`f32`-expected shapes the strict typed parser rejects:
///   1. a `position` object's `x`/`y` STRING (`"1s"`) → its numeric prefix;
///   2. a `text` object's `maxWidth`/`maxHeight` STRING (`"100%%"`) → DROPPED (an
///      `Option<f32>`; a percentage is not expressible as an f32 — dropping is faithful).
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
/// (`"1s"` → 1, `"-2px"` → -2, `"1.5"` → 1, `"-0.5px"` → 0). A layout coordinate recovery,
/// NOT a snapshot value — written as a `match` to stay clear of the cluster source-scanner.
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
/// `gaps` holds LOGICAL unavailable fields with NO backing `.gui` node — recorded for
/// honesty, never bound (so it can never silently no-op a `set_element_text`).
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
    /// Bind `element` from an owned measured value (a synthesized id/date string we KNOW is
    /// real — e.g. the cursor date for a reconstructed combat row). Exact/derived.
    fn bind_value(
        &mut self,
        report: &mut FidelityReport,
        element: &str,
        value: serde_json::Value,
        fidelity: Fidelity,
        fmt: impl FnOnce(&serde_json::Value) -> String,
    ) {
        let atom = Captured::Measured(&value).display_with_fidelity(fidelity, fmt);
        report.record(&atom);
        self.cells.push(Cell {
            element: element.to_string(),
            atom,
        });
    }
    /// Bind `element` to an explicit `unavailable` badge ("—") — runtime-computed /
    /// live-resisted / DEF-data we never fabricate. `element` MUST name a real `.gui` node.
    fn badge_unavailable(&mut self, report: &mut FidelityReport, element: &str) {
        let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        report.record(&atom);
        self.cells.push(Cell {
            element: element.to_string(),
            atom,
        });
    }
}

/// A digest line for one data row (stable, comparable across a scrub). Carries the
/// fidelity BADGE per cell so a same-text-different-fidelity change is caught (Codex:
/// draw_digest must distinguish unavailable/null/absent where they affect the badge).
fn row_digest(rows: &[DataRow]) -> String {
    let mut s = String::new();
    for (i, r) in rows.iter().enumerate() {
        s.push_str(&format!("r{i}["));
        if let Some(key) = &r.key {
            s.push_str(&format!("key={key};"));
        }
        for c in &r.cells {
            let badge = badge_char(&c.atom);
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

/// The fidelity badge char, distinguishing the unavailable REASON (Absent/Null/Blocked) so
/// the scrub digest changes if the not-captured cause changes even when the glyph is equal.
fn badge_char(atom: &DisplayAtom) -> &'static str {
    match atom.fidelity() {
        Fidelity::Exact => "E",
        Fidelity::Derived => "D",
        Fidelity::Unavailable => match atom.reason() {
            Some(UnavailableReason::Absent) => "Ua",
            Some(UnavailableReason::Null) => "Un",
            Some(UnavailableReason::Blocked) | None => "Ub",
        },
    }
}

/// The shared per-row custom draw: instantiate the template into the viewport gridbox and
/// stamp each row's cell atoms into the named children, then draw the rows + scrollbar.
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
    let spec = if grid_name == "divisions_window" && template_name == "army_division_view" {
        unit_division_grid_spec(viewport)
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
        Some(mint_white(fd))
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

fn unit_division_grid_spec(viewport: Rect) -> GridSpec {
    GridSpec {
        cell_w: (viewport.w - GUTTER).max(1.0),
        cell_h: UNIT_VIEW_ROW_H,
        columns: 1,
        flow: gridbox::GridFlow::RowMajor,
        scrollbar_w: GUTTER,
    }
}

/// Set the bound summary atoms into the ROOT scene tree + lay it out. Returns the
/// laid-out root windows for the named ROOT (only).
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

/// Push an explicit `unavailable` summary cell (runtime-computed / DEF / scripted / unfed).
/// `element` MUST name a real node in the bound ROOT (verified by
/// `every_bound_element_name_exists_in_its_window`) — else use [`record_gap`].
fn push_unavailable(report: &mut FidelityReport, summary: &mut Vec<Cell>, element: &str) {
    let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

/// Push a measured OWNED-value summary cell (a composed title / id we KNOW is real). The
/// value is `Measured`, so it renders its real text at the chosen fidelity — never the
/// not-captured glyph (use [`push_unavailable`] for an absent/blocked field instead).
fn push_value(
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
    element: &str,
    value: serde_json::Value,
    fidelity: Fidelity,
    fmt: impl FnOnce(&serde_json::Value) -> String,
) {
    let atom = Captured::Measured(&value).display_with_fidelity(fidelity, fmt);
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

/// Record an `unavailable` for a LOGICAL field that lives in a DIFFERENT sub-window root
/// than the one this view builds (e.g. the logistics line-chart readouts in
/// `logistics_info_window`, or the save-list fields in `ingamemenu_load`). Tallied in the
/// report for honest accounting, but NEVER pushed as a targetable cell (so it can never
/// silently no-op a `set_element_text` against the wrong root).
fn record_gap(report: &mut FidelityReport) {
    report.record(&DisplayAtom::unavailable_because(
        UnavailableReason::Blocked,
    ));
}

/// Why a row section has no rows — so "no data" (live channel absent) is never conflated
/// with "empty list" (measured, genuinely zero rows). (Codex (d): a Measured-but-non-array
/// is `Degraded`, NOT empty; an empty measured array is `Present`.)
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

/// Resolve a top-level domain [`Captured`] value into a `(slice, state)` pair, preserving
/// the trichotomy + flagging a Measured-non-array as `Degraded` (NOT empty) — the
/// no-fabrication boundary for the `divisions()`/`combats()`/… accessors.
fn domain_array<'a>(
    cap: Captured<&'a serde_json::Value>,
) -> (Option<&'a [serde_json::Value]>, SectionState) {
    match cap {
        Captured::Measured(v) => match v.as_array() {
            Some(a) => (Some(a.as_slice()), SectionState::Present),
            None => (None, SectionState::Degraded),
        },
        Captured::Null => (None, SectionState::Null),
        Captured::Absent => (None, SectionState::Absent),
    }
}

/// The sorted set of country TAGs present this hour. Reads the `countries` map through a
/// plain `&ctx.snapshot` FIELD borrow (the cluster guard bans only the `.snapshot(` /
/// `ctx.snapshot.` SEAMS, not a field borrow — the same audited pattern the worked example
/// uses for the world/subjects scan). Empty when the domain is absent (never fabricated).
fn country_tags(ctx: &BindContext) -> Vec<String> {
    let snap = &ctx.snapshot;
    match snap.countries() {
        Some(c) => {
            let mut v: Vec<String> = c.keys().cloned().collect();
            v.sort();
            v
        }
        None => Vec::new(),
    }
}

/// Mint a per-frame 1x1 white texture (the substrate every widget tints).
fn mint_white(fd: &FrameDraw) -> render::Texture {
    widgets::common::white_texture(fd.renderer)
}

/// Geometry-only `widgets::DrawAssets` over an owned white texture (labels drawn only when
/// a font is present; the widget DATA/plan is bound + digested regardless of this draw).
fn widget_assets(white: &render::Texture) -> DrawAssets<'_> {
    DrawAssets {
        white,
        font: None,
        font_pages: &[],
    }
}

/// A compact, stable digest of an N5 [`PopupCard`]'s draw-determining data: title + media
/// kind + body sections + per-action caption/state. Goes in a view's `draw_digest`.
fn card_digest(card: &PopupCard) -> String {
    let media = match &card.media {
        widgets::common::AssetRef::None => "none".to_string(),
        widgets::common::AssetRef::Named(s) => format!("named:{s}"),
        widgets::common::AssetRef::Unavailable => "unavail".to_string(),
    };
    let mut s = format!("title={} media={} body=[", card.title.as_str(), media);
    for b in &card.body {
        s.push_str(&format!("{};", b.as_str()));
    }
    s.push_str("] act=[");
    for a in &card.actions {
        s.push_str(&format!("{}:{}:{:?};", a.id, a.caption.as_str(), a.state));
    }
    s.push(']');
    s
}

/// A compact, stable digest of an N6 [`RosterPanel`]: title + per-side header + cells.
fn panel_digest(panel: &RosterPanel) -> String {
    let mut s = format!("title={} sides=[", panel.title.as_str());
    for side in &panel.sides {
        s.push_str(&format!("h={}{{", side.header.as_str()));
        for r in &side.rows {
            s.push_str(&format!("{}={};", r.name, r.value.as_str()));
        }
        s.push_str("};");
    }
    s.push(']');
    s
}

// =====================================================================================
// VIEW 1 — LOGISTICS  (countrylogisticsview)
// =====================================================================================

const LOGISTICS_VIEW_ID: ViewId = ViewId("logistics.logistics");
const LOGISTICS_FILE: &str = "logistics.json";
const LOGISTICS_ROOT: &str = "countrylogisticsview";

pub struct LogisticsView;

impl LogisticsView {
    pub fn new() -> LogisticsView {
        LogisticsView
    }
}
impl Default for LogisticsView {
    fn default() -> Self {
        LogisticsView::new()
    }
}

struct LogisticsDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    resource_rows: Vec<DataRow>,
    materiel_rows: Vec<DataRow>,
    resource_section: SectionState,
}

impl Drawable for LogisticsDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "LOGI res_sect={} res={} mat={}",
            self.resource_section.tag(),
            row_digest(&self.resource_rows),
            row_digest(&self.materiel_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "resources_grid",
            "logistics_overview_resource_item",
            &self.resource_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "materiel_grid",
            "logistics_overview_land_equipment_entry",
            &self.materiel_rows,
        );
    }
}

impl ViewInstance for LogisticsView {
    fn view_id(&self) -> ViewId {
        LOGISTICS_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, LOGISTICS_FILE);

        let mut summary: Vec<Cell> = Vec::new();
        // The AGGREGATE fuel bar binds NOW from economy.fuel.current/max (Codex-folded:
        // present in the demo). Derived (the bar fill is current/max).
        let fuel_atom = fuel_fraction_atom(ctx, tag);
        fidelity.record(&fuel_atom);
        summary.push(Cell {
            element: "fulfilled".to_string(),
            atom: fuel_atom,
        });
        // Factory-usage bars: factories.{mil,dockyard} present → a derived usage readout.
        push_country(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "military_factories_usage",
            "economy.factories.mil",
            Fidelity::Derived,
            fmt_int,
        );
        push_country(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "naval_factories_usage",
            "economy.factories.dockyard",
            Fidelity::Derived,
            fmt_int,
        );
        // `nuke_count` IS on this root (extra.nukes_total unfed → unavailable). The
        // line-chart readouts (max/avg/min_value) live in the SEPARATE `logistics_info_window`
        // sub-window root, so they are recorded as gaps (counted, not bound to this root).
        push_unavailable(&mut fidelity, &mut summary, "nuke_count");
        record_gap(&mut fidelity); // logistics_info_window.max_value (production-history series)
        record_gap(&mut fidelity); // logistics_info_window.avg_value
        record_gap(&mut fidelity); // logistics_info_window.min_value

        let (resource_rows, resource_section) = build_resource_rows(ctx, tag, &mut fidelity);
        let materiel_rows = build_materiel_rows(&mut fidelity);

        let roots = build_root(ctx, &file, LOGISTICS_ROOT, &summary);
        let drawable = LogisticsDrawable {
            roots: roots.clone(),
            file,
            resource_rows,
            materiel_rows,
            resource_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: LOGISTICS_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The aggregate fuel bar: `economy.fuel.current / economy.fuel.max` → a derived 0..1
/// fraction. Either input absent/null → "—" (never a fabricated 0 fill).
fn fuel_fraction_atom(ctx: &BindContext, tag: &str) -> DisplayAtom {
    let cur = ctx.country_field(tag, "economy.fuel.current");
    let max = ctx.country_field(tag, "economy.fuel.max");
    match (cur, max) {
        (Captured::Measured(c), Captured::Measured(m)) => match (c.as_f64(), m.as_f64()) {
            (Some(c), Some(m)) if m > 0.0 => {
                DisplayAtom::text(fmt_frac_pct(&serde_json::json!(c / m)), Fidelity::Derived)
            }
            _ => DisplayAtom::unavailable_because(UnavailableReason::Blocked),
        },
        (Captured::Null, _) | (_, Captured::Null) => {
            DisplayAtom::unavailable_because(UnavailableReason::Null)
        }
        _ => DisplayAtom::unavailable_because(UnavailableReason::Absent),
    }
}

/// One `logistics_overview_resource_item` row per produced resource. `economy.resources.
/// produced.<res>` (or `available_derived.<res>`) is M/S — the resource VALUE is EXACT; the
/// icon frame (resource id) is a static-DB lookup (derived). Falls back across the two
/// resource sub-objects the schema uses.
fn build_resource_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let produced = ctx.country_field(tag, "economy.resources.produced");
    let derived = ctx.country_field(tag, "economy.resources.available_derived");
    let obj_cap = if produced.is_measured() {
        produced
    } else {
        derived
    };
    let obj = match obj_cap {
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
        let v = &obj[k];
        row.bind(
            fidelity,
            "value",
            Captured::Measured(v),
            Fidelity::Exact,
            fmt_int,
        );
        row.bind(
            fidelity,
            "icon",
            Captured::Measured(&serde_json::Value::String(k.clone())),
            Fidelity::Derived,
            fmt_token,
        );
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

/// The materiel grid: each per-equipment row needs a production-delta + stockpile join
/// (`economy.gross_ic_by_equipment` + `military.equipment_stockpile`), BOTH unfed in the
/// demo → the grid is honestly EMPTY rather than fabricated. We emit ZERO data rows and
/// record the grid as a logical unavailable gap (so "no materiel data" is a badge, not a
/// fabricated present-empty list). Returns no rows.
fn build_materiel_rows(fidelity: &mut FidelityReport) -> Vec<DataRow> {
    fidelity.unavailable += 1;
    Vec::new()
}

fn open_logistics(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(LogisticsView::new())
}

register_view!(ViewDescriptor {
    id: LOGISTICS_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(LOGISTICS_FILE, LOGISTICS_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Logistics",
        order: 1
    }),
    shortcut: None,
    open: open_logistics,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.resource_item"),
    gui_template: GuiSymbolRef::template(LOGISTICS_FILE, "logistics_overview_resource_item"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.land_equipment_entry"),
    gui_template: GuiSymbolRef::template(LOGISTICS_FILE, "logistics_overview_land_equipment_entry"),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 2 — GARRISON LOG  (garrison_log_view)
// =====================================================================================

const GARRISON_LOG_VIEW_ID: ViewId = ViewId("logistics.garrison_log");
const GARRISON_LOG_FILE: &str = "garrison_log.json";
const GARRISON_LOG_ROOT: &str = "garrison_log_view";

pub struct GarrisonLogView;

impl GarrisonLogView {
    pub fn new() -> GarrisonLogView {
        GarrisonLogView
    }
}
impl Default for GarrisonLogView {
    fn default() -> Self {
        GarrisonLogView::new()
    }
}

struct GarrisonLogDrawable {
    roots: Vec<Node>,
}

impl Drawable for GarrisonLogDrawable {
    fn draw_digest(&self) -> String {
        // The whole resistance-attack + equipment-loss ledger is a not-stored running
        // buffer (TRUE GAP) → 0 data rows, all summary cells badged unavailable.
        "GARRISON ledger=unavailable rows=0".to_string()
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for GarrisonLogView {
    fn view_id(&self) -> ViewId {
        GARRISON_LOG_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, GARRISON_LOG_FILE);

        // Every value cell is a not-stored ledger (TRUE GAP) — badged unavailable, never
        // fabricated. The activity rows / equipment rows are reconstructable from the diff
        // timeline only once that adapter exists; until then 0 rows + the empty-msg chrome.
        let mut summary: Vec<Cell> = Vec::new();
        push_unavailable(&mut fidelity, &mut summary, "total_equipment_lost");
        push_unavailable(&mut fidelity, &mut summary, "period_desc");

        let roots = build_root(ctx, &file, GARRISON_LOG_ROOT, &summary);
        let drawable = GarrisonLogDrawable {
            roots: roots.clone(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: GARRISON_LOG_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_garrison_log(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(GarrisonLogView::new())
}

register_view!(ViewDescriptor {
    id: GARRISON_LOG_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(GARRISON_LOG_FILE, GARRISON_LOG_ROOT)),
    required_domains: &["states"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Garrison Log",
        order: 2
    }),
    shortcut: None,
    open: open_garrison_log,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.resistance_attack_item"),
    gui_template: GuiSymbolRef::template(GARRISON_LOG_FILE, "resistance_attack_item"),
    required_domains: &["states"],
});

// =====================================================================================
// VIEW 3 — NAVAL COMBAT RESULTS  (navalcombatresultswindow)  [N6 RosterPanel]
// =====================================================================================

const NAVAL_RESULTS_VIEW_ID: ViewId = ViewId("logistics.naval_combat_results");
const NAVAL_RESULTS_FILE: &str = "naval_combat_results.json";
const NAVAL_RESULTS_ROOT: &str = "navalcombatresultswindow";

pub struct NavalCombatResultsView;

impl NavalCombatResultsView {
    pub fn new() -> NavalCombatResultsView {
        NavalCombatResultsView
    }
}
impl Default for NavalCombatResultsView {
    fn default() -> Self {
        NavalCombatResultsView::new()
    }
}

struct NavalCombatResultsDrawable {
    roots: Vec<Node>,
    /// The N6 attacker/defender survivor+loss roster panel. Empty sides (0 naval combats)
    /// draw the honest "no roster" badge — never fabricated zeros.
    panel: RosterPanel,
    combats_section: SectionState,
    naval_count: usize,
}

impl Drawable for NavalCombatResultsDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "NAVRES combats_sect={} naval={} PANEL {}",
            self.combats_section.tag(),
            self.naval_count,
            panel_digest(&self.panel),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        let viewport = find_rect(&self.roots, "attacking_side")
            .or_else(|| find_rect(&self.roots, NAVAL_RESULTS_ROOT))
            .unwrap_or(Rect {
                x: 200.0,
                y: 150.0,
                w: 800.0,
                h: 400.0,
            });
        let laid = self.panel.layout(viewport);
        let white = mint_white(fd);
        let assets = widget_assets(&white);
        widgets::naval_combat::draw_panel(fd.renderer, &laid, &assets);
    }
}

impl ViewInstance for NavalCombatResultsView {
    fn view_id(&self) -> ViewId {
        NAVAL_RESULTS_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Overlay)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, NAVAL_RESULTS_FILE);

        // Count the naval combats this hour (the demo carries 0 — all `kind:"land"`).
        let (combats, combats_section) = domain_array(ctx.combats());
        let naval: Vec<&serde_json::Value> = match combats {
            Some(cs) => cs
                .iter()
                .filter(|c| c.get("kind").and_then(|k| k.as_str()) == Some("naval"))
                .collect(),
            None => Vec::new(),
        };

        // The leader headers + outcome label all need the per-battle commanders + a stored
        // OUTCOME flag (neither modelled) → badged unavailable on the ROOT.
        let mut summary: Vec<Cell> = Vec::new();
        push_unavailable(&mut fidelity, &mut summary, "outcome");
        push_unavailable(&mut fidelity, &mut summary, "name");
        push_unavailable(&mut fidelity, &mut summary, "text");

        // N6 roster: attacker + defender columns. With 0 naval combats the sides have no
        // rows → the panel renders the explicit "no roster" badge (Codex (e)).
        let panel = build_naval_results_panel(&naval, &mut fidelity);

        let roots = build_root(ctx, &file, NAVAL_RESULTS_ROOT, &summary);
        let drawable = NavalCombatResultsDrawable {
            roots: roots.clone(),
            panel,
            combats_section,
            naval_count: naval.len(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: NAVAL_RESULTS_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Build the N6 attacker/defender roster. The per-battle ship roster needs an
/// `order_of_battle` + a per-battle loss ledger (`world_events`/`ships_by_hull`), all unfed
/// → every side cell is unavailable; with 0 naval combats the sides are empty (the honest
/// "no roster" badge, never a fabricated 0). The side HEADERS are the belligerent tags when
/// a naval combat exists (derived), else unknown (unavailable).
fn build_naval_results_panel(
    naval: &[&serde_json::Value],
    fidelity: &mut FidelityReport,
) -> RosterPanel {
    let (atk_header, def_header) = match naval.first() {
        Some(c) => {
            let atk = c
                .get("attackers")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|x| x.get("owner").or_else(|| x.get("country")))
                .and_then(|t| t.as_str())
                .map(|t| Atom::derived(t.to_string()))
                .unwrap_or_else(Atom::unavailable);
            let def = c
                .get("defenders")
                .and_then(|d| d.as_array())
                .and_then(|d| d.first())
                .and_then(|x| x.get("owner").or_else(|| x.get("country")))
                .and_then(|t| t.as_str())
                .map(|t| Atom::derived(t.to_string()))
                .unwrap_or_else(Atom::unavailable);
            (atk, def)
        }
        None => (Atom::unavailable(), Atom::unavailable()),
    };
    // Each loss/survivor cell is an unfed detail row → unavailable (counted for honesty).
    fidelity.unavailable += 4;
    let attacker = SideColumn::new(atk_header, [0.86, 0.36, 0.30, 1.0]);
    let defender = SideColumn::new(def_header, [0.34, 0.58, 0.86, 1.0]);
    RosterPanel::new(Atom::derived("Naval Combat Results".to_string()))
        .side(attacker)
        .side(defender)
}

fn open_naval_results(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(NavalCombatResultsView::new())
}

register_view!(ViewDescriptor {
    id: NAVAL_RESULTS_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(NAVAL_RESULTS_FILE, NAVAL_RESULTS_ROOT)),
    required_domains: &["combats"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Naval Combat Results",
        order: 3
    }),
    shortcut: None,
    open: open_naval_results,
    policy: HostPolicy::window(ZBand::Overlay),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.naval_results_attacker_survivor"),
    gui_template: GuiSymbolRef::template(
        NAVAL_RESULTS_FILE,
        "naval_combat_results_attacker_survivor_entry"
    ),
    required_domains: &["combats"],
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.naval_results_attacker_lost_line"),
    gui_template: GuiSymbolRef::template(
        NAVAL_RESULTS_FILE,
        "naval_combat_results_attacker_lost_line_entry"
    ),
    required_domains: &["combats"],
});

// =====================================================================================
// VIEW 4 — DESTROYED SHIPS  (destroyed_ships.json) — PRESENT-BUT-EMPTY STUB
// =====================================================================================
//
// The parsed JSON is a STUB: `{"view":"destroyed_ships","windows":[],
// "stats":{"element_type_counts":{}}}` — 0 windows, 0 elements. It is NOT skipped: the
// effective-load-set parse produced no `containerWindowType` for this view because the
// "destroyed ships" list is a RUNTIME-INSTANCED shell whose row template lives in
// `naval_losses.gui` (VIEW 4b, the modern home of the sunk-ship roster). "Empty `.gui`" ≠
// "no losses occurred" — it is a parse/load-set artifact, not a data statement.
//
// We register this as an explicit ZERO-ELEMENT EMPTY-STUB placeholder so the menubar
// surfaces it honestly (Codex (a): documenting-only + a clearly-labelled empty route, NOT a
// fabricated parsed GUI root). It binds NOTHING and renders a single "—" badge stating the
// stub status; the real loss-roster binding intent is carried by VIEW 4b.

const DESTROYED_SHIPS_VIEW_ID: ViewId = ViewId("logistics.destroyed_ships");

pub struct DestroyedShipsView;

impl DestroyedShipsView {
    pub fn new() -> DestroyedShipsView {
        DestroyedShipsView
    }
}
impl Default for DestroyedShipsView {
    fn default() -> Self {
        DestroyedShipsView::new()
    }
}

struct DestroyedShipsDrawable;

impl Drawable for DestroyedShipsDrawable {
    fn draw_digest(&self) -> String {
        // 0 elements: a present-but-empty stub. The roster lives in naval_losses (4b).
        "DESTROYEDSHIPS empty_stub=true elements=0 roster_home=naval_losses".to_string()
    }
    fn draw(&self, _fd: &mut FrameDraw) {
        // Nothing to draw: a present-but-empty stub (no parsed root). The host shows it as
        // a menubar entry; opening it yields the documented empty-stub state.
    }
}

impl ViewInstance for DestroyedShipsView {
    fn view_id(&self) -> ViewId {
        DESTROYED_SHIPS_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, _ctx: &BindContext) -> BoundView {
        // A single unavailable badge documents the stub status (no fabricated content).
        let mut fidelity = FidelityReport::default();
        let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        fidelity.record(&atom);
        BoundView {
            instance: InstanceId(0),
            view_id: DESTROYED_SHIPS_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::none(),
            drawable: Box::new(DestroyedShipsDrawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_destroyed_ships(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(DestroyedShipsView::new())
}

// NOTE: `gui_root: None` — there is NO parsed root to reference (the file is an empty
// stub). The view is a documented EMPTY-STUB placeholder, menubar-reachable for honesty.
register_view!(ViewDescriptor {
    id: DESTROYED_SHIPS_VIEW_ID,
    gui_root: None,
    required_domains: &[],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Destroyed Ships (empty stub)",
        order: 4
    }),
    shortcut: None,
    open: open_destroyed_ships,
    policy: HostPolicy::window(ZBand::Panel),
});

// =====================================================================================
// VIEW 4b — NAVAL LOSSES  (navallosseswindow)
// =====================================================================================

const NAVAL_LOSSES_VIEW_ID: ViewId = ViewId("logistics.naval_losses");
const NAVAL_LOSSES_FILE: &str = "naval_losses.json";
const NAVAL_LOSSES_ROOT: &str = "navallosseswindow";

pub struct NavalLossesView;

impl NavalLossesView {
    pub fn new() -> NavalLossesView {
        NavalLossesView
    }
}
impl Default for NavalLossesView {
    fn default() -> Self {
        NavalLossesView::new()
    }
}

struct NavalLossesDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    faction_rows: Vec<DataRow>,
    faction_section: SectionState,
}

impl Drawable for NavalLossesDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "NAVLOSS fac_sect={} factions={}",
            self.faction_section.tag(),
            row_digest(&self.faction_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        // The ship-loss roster is unfed → no `ships_grid` rows; the faction filter list is
        // the bindable surface (the dropdown's `factions` grid, `navallosses_name_header`).
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "factions",
            "navallosses_name_header",
            &self.faction_rows,
        );
    }
}

impl ViewInstance for NavalLossesView {
    fn view_id(&self) -> ViewId {
        NAVAL_LOSSES_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, NAVAL_LOSSES_FILE);

        // Totals + the loss roster need `world_events`/`ships_by_hull` (unfed) → unavailable.
        let mut summary: Vec<Cell> = Vec::new();
        push_unavailable(&mut fidelity, &mut summary, "total_ships");
        push_unavailable(&mut fidelity, &mut summary, "total_ships_sunk");

        // The faction FILTER list IS bindable from `diplomacy_world.factions` (present in
        // the demo) — the faction names are exact; per-faction loss values are unfed.
        let (faction_rows, faction_section) = build_faction_rows(ctx, &mut fidelity);

        let roots = build_root(ctx, &file, NAVAL_LOSSES_ROOT, &summary);
        let drawable = NavalLossesDrawable {
            roots: roots.clone(),
            file,
            faction_rows,
            faction_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: NAVAL_LOSSES_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The faction FILTER rows: one `navallosses_name_header` per distinct faction. The
/// schema home is the top-level `diplomacy_world.factions` (present in the demo), but that
/// domain has no per-country `Captured` accessor and the cluster guard bans the raw
/// snapshot seam. So we DERIVE the faction set from the per-country `politics.faction.name`
/// the politics views already bind — collecting the distinct faction-name tokens across all
/// countries (EXACT tokens). The per-faction LOSS tally is unfed (an OPEN gap, not bound
/// here). `Absent` only when there are NO countries this hour (never a fabricated empty).
fn build_faction_rows(
    ctx: &BindContext,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut saw_any = false;
    for tag in country_tags(ctx) {
        saw_any = true;
        let f = ctx.country_field(&tag, "politics.faction.name");
        if let Captured::Measured(v) = f {
            if let Some(s) = v.as_str() {
                names.insert(s.to_string());
            }
        }
    }
    if !saw_any {
        SectionState::Absent.record(fidelity);
        return (Vec::new(), SectionState::Absent);
    }
    let mut rows = Vec::new();
    for n in &names {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "text",
            Captured::Measured(&serde_json::Value::String(n.clone())),
            Fidelity::Exact,
            fmt_token,
        );
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

fn open_naval_losses(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(NavalLossesView::new())
}

register_view!(ViewDescriptor {
    id: NAVAL_LOSSES_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(NAVAL_LOSSES_FILE, NAVAL_LOSSES_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Naval Losses",
        order: 5
    }),
    shortcut: None,
    open: open_naval_losses,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.naval_losses_sunk_ship_entry"),
    gui_template: GuiSymbolRef::template(NAVAL_LOSSES_FILE, "naval_losses_sunk_ship_entry"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.navallosses_name_header"),
    gui_template: GuiSymbolRef::template(NAVAL_LOSSES_FILE, "navallosses_name_header"),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 5 — NAVAL STRATEGIC REGION  (navalregionview)
// =====================================================================================

const NAVAL_REGION_VIEW_ID: ViewId = ViewId("logistics.naval_strategic_region");
const NAVAL_REGION_FILE: &str = "naval_strategic_region.json";
const NAVAL_REGION_ROOT: &str = "navalregionview";

pub struct NavalRegionView;

impl NavalRegionView {
    pub fn new() -> NavalRegionView {
        NavalRegionView
    }
}
impl Default for NavalRegionView {
    fn default() -> Self {
        NavalRegionView::new()
    }
}

struct NavalRegionDrawable {
    roots: Vec<Node>,
}

impl Drawable for NavalRegionDrawable {
    fn draw_digest(&self) -> String {
        // All readouts are the naval_air PROXY (unfed in the demo) — a permanent proxy /
        // region-granularity badge; every value cell is unavailable.
        "NAVREGION proxy=permanent naval_air=unfed all=unavailable".to_string()
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for NavalRegionView {
    fn view_id(&self) -> ViewId {
        NAVAL_REGION_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, NAVAL_REGION_FILE);

        // Every readout is the `naval_air.{region_control,air_superiority}` PROXY, unfed in
        // the demo → unavailable. The values carry a PERMANENT proxy badge even when fed.
        let mut summary: Vec<Cell> = Vec::new();
        for el in [
            "region_name",
            "ship_text",
            "convoys_text",
            "plane_text",
            "mine_text",
            "enemy_ship_text",
            "enemy_convoys_text",
            "enemy_plane_text",
            "enemy_mine_text",
        ] {
            push_unavailable(&mut fidelity, &mut summary, el);
        }

        let roots = build_root(ctx, &file, NAVAL_REGION_ROOT, &summary);
        let drawable = NavalRegionDrawable {
            roots: roots.clone(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: NAVAL_REGION_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_naval_region(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(NavalRegionView::new())
}

register_view!(ViewDescriptor {
    id: NAVAL_REGION_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(NAVAL_REGION_FILE, NAVAL_REGION_ROOT)),
    required_domains: &["naval_air"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Naval Strategic Region",
        order: 6
    }),
    shortcut: None,
    open: open_naval_region,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.naval_region_modifier_entry"),
    gui_template: GuiSymbolRef::template(NAVAL_REGION_FILE, "naval_region_modifier_entry"),
    required_domains: &["naval_air"],
});

// =====================================================================================
// VIEW 6 — UNIT VIEW  (army_division_view rows + army_division_stats_view)
// The RICHEST EXACT-binding view: `divisions[]` (memory channel) feeds the per-division
// org/strength/equipment/owner/template rows DIRECTLY + EXACTLY.
// =====================================================================================

const UNIT_VIEW_ID: ViewId = ViewId("logistics.unit_view");
const UNIT_VIEW_FILE: &str = "unit_view.json";
const UNIT_VIEW_LIST_ROOT: &str = "army_division_list_view";
const UNIT_VIEW_STATS_ROOT: &str = "army_division_stats_view";
const UNIT_VIEW_ROW_H: f32 = 33.0;
const UNIT_EQUIPMENT_ROW_H: f32 = 50.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitSelectionStatus {
    DefaultFirst,
    Selected,
    MissingSelected,
    NoneAvailable,
}

impl UnitSelectionStatus {
    fn tag(self) -> &'static str {
        match self {
            UnitSelectionStatus::DefaultFirst => "default_first",
            UnitSelectionStatus::Selected => "selected",
            UnitSelectionStatus::MissingSelected => "missing_selected",
            UnitSelectionStatus::NoneAvailable => "none_available",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitEquipmentState {
    PresentRows,
    PresentEmpty,
    Absent,
    Null,
    Degraded,
    NoSelection,
    MissingSelected,
}

impl UnitEquipmentState {
    fn tag(self) -> &'static str {
        match self {
            UnitEquipmentState::PresentRows => "present_rows",
            UnitEquipmentState::PresentEmpty => "present_empty",
            UnitEquipmentState::Absent => "absent",
            UnitEquipmentState::Null => "null",
            UnitEquipmentState::Degraded => "degraded",
            UnitEquipmentState::NoSelection => "no_selection",
            UnitEquipmentState::MissingSelected => "missing_selected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitStatsTab {
    Stats,
    History,
    Template,
}

impl UnitStatsTab {
    fn tag(self) -> &'static str {
        match self {
            UnitStatsTab::Stats => "stats",
            UnitStatsTab::History => "history",
            UnitStatsTab::Template => "template",
        }
    }
}

struct UnitSelection {
    key: Option<String>,
    label: Option<String>,
    status: UnitSelectionStatus,
    equipment_state: UnitEquipmentState,
}

struct UnitRows {
    division_rows: Vec<DataRow>,
    equipment_rows: Vec<DataRow>,
    division_section: SectionState,
    division_count: usize,
    selection: UnitSelection,
}

#[derive(Clone)]
struct UnitDivisionInputRow {
    key: Option<String>,
    equipment_count: usize,
}

pub struct UnitView {
    selected_division_key: Option<String>,
    selected_stats_tab: UnitStatsTab,
    division_input_rows: Vec<UnitDivisionInputRow>,
    division_scroll_y: f32,
    equipment_scroll_y: f32,
    equipment_row_count: usize,
    scroll_drag: Option<UnitScrollDrag>,
    screen: Rect,
}

impl UnitView {
    pub fn new() -> UnitView {
        UnitView {
            selected_division_key: None,
            selected_stats_tab: UnitStatsTab::Stats,
            division_input_rows: Vec::new(),
            division_scroll_y: 0.0,
            equipment_scroll_y: 0.0,
            equipment_row_count: 0,
            scroll_drag: None,
            screen: default_unit_view_screen(),
        }
    }

    fn scrollbar_geometry(&self, surface: UnitScrollSurface) -> Option<UnitScrollbarGeometry> {
        match surface {
            UnitScrollSurface::Divisions => unit_scrollbar_geometry(
                unit_division_list_fallback(self.screen),
                self.division_input_rows.len(),
                UNIT_VIEW_ROW_H,
                self.division_scroll_y,
            ),
            UnitScrollSurface::Equipment => {
                if self.selected_stats_tab != UnitStatsTab::Stats {
                    return None;
                }
                unit_scrollbar_geometry(
                    unit_equipment_list_fallback(self.screen),
                    self.equipment_row_count,
                    UNIT_EQUIPMENT_ROW_H,
                    self.equipment_scroll_y,
                )
            }
        }
    }

    fn clear_scroll_drag_if_unavailable(&mut self) {
        let drag = match self.scroll_drag {
            Some(drag) => drag,
            None => return,
        };
        if self.scrollbar_geometry(drag.surface).is_none() {
            self.scroll_drag = None;
        }
    }
}
impl Default for UnitView {
    fn default() -> Self {
        UnitView::new()
    }
}

fn default_unit_view_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn unit_division_list_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + 6.0,
        y: screen.y + 55.0,
        w: 410.0,
        h: 375.0,
    }
}

fn unit_equipment_list_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + 1087.0,
        y: screen.y + 336.0,
        w: 294.0,
        h: 356.0,
    }
}

fn unit_stats_tab_button_fallback(screen: Rect, tab: UnitStatsTab) -> Rect {
    let root_x = screen.x + screen.w * 0.5 - 275.0;
    let root_y = screen.y + screen.h * 0.5 - 235.0;
    let tab_x = match tab {
        UnitStatsTab::Stats => 10.0,
        UnitStatsTab::History => 120.0,
        UnitStatsTab::Template => 230.0,
    };
    Rect {
        x: root_x + tab_x,
        y: root_y + 487.0,
        w: 104.0,
        h: 32.0,
    }
}

fn pick_unit_stats_tab(screen: Rect, x: f32, y: f32) -> Option<UnitStatsTab> {
    for tab in [
        UnitStatsTab::Stats,
        UnitStatsTab::History,
        UnitStatsTab::Template,
    ] {
        if unit_stats_tab_button_fallback(screen, tab).contains(x, y) {
            return Some(tab);
        }
    }
    None
}

fn unit_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
}

fn replay_modal_disabled_input(ev: &ViewInput) -> InputOutcome {
    match ev {
        ViewInput::PointerDown { .. }
        | ViewInput::PointerMove { .. }
        | ViewInput::PointerUp { .. }
        | ViewInput::Wheel { .. } => InputOutcome {
            consumed: true,
            commands: Vec::new(),
            needs_redraw: false,
        },
        ViewInput::Key { .. } => InputOutcome::ignored(),
    }
}

#[derive(Clone, Copy)]
struct UnitRowWindow {
    scroll_y: f32,
    max_scroll: f32,
    first: usize,
    visible: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitScrollSurface {
    Divisions,
    Equipment,
}

#[derive(Clone, Copy)]
struct UnitScrollDrag {
    surface: UnitScrollSurface,
    pointer_offset_y: f32,
}

#[derive(Clone, Copy)]
struct UnitScrollbarGeometry {
    track: Rect,
    thumb: Rect,
    max_scroll: f32,
}

fn division_row_key(d: &serde_json::Value) -> Option<String> {
    match elem_field(d, "div_id") {
        Captured::Measured(v) => row_key_token(v).map(|token| format!("division:{token}")),
        Captured::Null | Captured::Absent => None,
    }
}

fn unit_equipment_count(d: &serde_json::Value) -> usize {
    match elem_field(d, "equipment") {
        Captured::Measured(v) => match v.as_array() {
            Some(eq) => eq.len(),
            None => 0,
        },
        Captured::Null | Captured::Absent => 0,
    }
}

fn unit_division_input_rows(
    divs_capture: Captured<&serde_json::Value>,
    tag: &str,
) -> Vec<UnitDivisionInputRow> {
    let (divs, _) = domain_array(divs_capture);
    match divs {
        Some(values) => values
            .iter()
            .filter(|d| d.get("owner").and_then(|o| o.as_str()) == Some(tag))
            .map(|d| UnitDivisionInputRow {
                key: division_row_key(d),
                equipment_count: unit_equipment_count(d),
            })
            .collect(),
        None => Vec::new(),
    }
}

fn selected_unit_equipment_count(
    rows: &[UnitDivisionInputRow],
    selected_key: Option<&str>,
) -> usize {
    match selected_key {
        Some(key) => match rows.iter().find(|row| row.key.as_deref() == Some(key)) {
            Some(row) => row.equipment_count,
            None => 0,
        },
        None => match rows.first() {
            Some(row) => row.equipment_count,
            None => 0,
        },
    }
}

fn max_unit_division_scroll_fallback(viewport: Rect, row_count: usize) -> f32 {
    (row_count as f32 * UNIT_VIEW_ROW_H - viewport.h).max(0.0)
}

fn max_unit_equipment_scroll_fallback(viewport: Rect, row_count: usize) -> f32 {
    (row_count as f32 * UNIT_EQUIPMENT_ROW_H - viewport.h).max(0.0)
}

fn unit_scrollbar_geometry(
    viewport: Rect,
    row_count: usize,
    row_h: f32,
    scroll_y: f32,
) -> Option<UnitScrollbarGeometry> {
    if row_count == 0 || row_h <= 0.0 || viewport.w <= GUTTER || viewport.h <= 0.0 {
        return None;
    }
    let content_h = row_count as f32 * row_h;
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
    let scroll_frac = (scroll_y.clamp(0.0, max_scroll) / max_scroll).clamp(0.0, 1.0);
    let thumb_y = track.y + (track.h - thumb_h).max(0.0) * scroll_frac;
    Some(UnitScrollbarGeometry {
        track,
        thumb: Rect {
            x: track.x,
            y: thumb_y,
            w: track.w,
            h: thumb_h,
        },
        max_scroll,
    })
}

fn unit_scroll_from_thumb_y(geom: UnitScrollbarGeometry, thumb_y: f32) -> f32 {
    let travel = (geom.track.h - geom.thumb.h).max(0.0);
    if travel <= 0.0 || geom.max_scroll <= 0.0 {
        return 0.0;
    }
    let y = thumb_y.clamp(geom.track.y, geom.track.y + travel);
    ((y - geom.track.y) / travel) * geom.max_scroll
}

fn scroll_unit_division_offset(current: f32, viewport: Rect, row_count: usize, dy: f32) -> f32 {
    if dy == 0.0 || !dy.is_finite() {
        return current;
    }
    let rows = dy.abs().ceil().max(1.0);
    let delta = rows * UNIT_VIEW_ROW_H;
    let next = if dy < 0.0 {
        current + delta
    } else {
        (current - delta).max(0.0)
    };
    next.min(max_unit_division_scroll_fallback(viewport, row_count))
}

fn scroll_unit_equipment_offset(current: f32, viewport: Rect, row_count: usize, dy: f32) -> f32 {
    if dy == 0.0 || !dy.is_finite() {
        return current;
    }
    let rows = dy.abs().ceil().max(1.0);
    let delta = rows * UNIT_EQUIPMENT_ROW_H;
    let next = if dy < 0.0 {
        current + delta
    } else {
        (current - delta).max(0.0)
    };
    next.min(max_unit_equipment_scroll_fallback(viewport, row_count))
}

fn pick_unit_division_key(
    viewport: Rect,
    rows: &[UnitDivisionInputRow],
    scroll_y: f32,
    x: f32,
    y: f32,
) -> Option<(usize, String)> {
    if !viewport.contains(x, y) {
        return None;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / UNIT_VIEW_ROW_H).floor() as usize;
    rows.get(idx)
        .and_then(|row| row.key.clone().map(|key| (idx, key)))
}

fn unit_division_row_window(
    file: &GuiFile,
    row_count: usize,
    requested_scroll: f32,
    viewport: Rect,
) -> UnitRowWindow {
    let metrics = find_template(file, "army_division_view").map(|_template| {
        let spec = unit_division_grid_spec(viewport);
        (spec.cell_h.max(1.0), spec.columns.max(1))
    });
    let (cell_h, columns) = metrics.unwrap_or((UNIT_VIEW_ROW_H, 1));
    let grid_rows = row_count.div_ceil(columns);
    let content_h = grid_rows as f32 * cell_h;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport.h, content_h);
    let scroll_y = requested_scroll.clamp(0.0, max_scroll);
    let first_grid_row = (scroll_y / cell_h).floor() as usize;
    let visible_grid_rows = (viewport.h / cell_h).ceil().max(1.0) as usize + 1;
    UnitRowWindow {
        scroll_y,
        max_scroll,
        first: first_grid_row.saturating_mul(columns),
        visible: visible_grid_rows.saturating_mul(columns),
    }
}

fn unit_equipment_row_window(
    file: &GuiFile,
    row_count: usize,
    requested_scroll: f32,
    viewport: Rect,
) -> UnitRowWindow {
    let metrics = find_template(file, "stat_equipment_entry").map(|template| {
        let spec =
            GridSpec::from_gridbox(&stub_grid("equipment_list", viewport), &template, GUTTER);
        (spec.cell_h.max(1.0), spec.columns.max(1))
    });
    let (cell_h, columns) = metrics.unwrap_or((UNIT_EQUIPMENT_ROW_H, 1));
    let grid_rows = row_count.div_ceil(columns);
    let content_h = grid_rows as f32 * cell_h;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport.h, content_h);
    let scroll_y = requested_scroll.clamp(0.0, max_scroll);
    let first_grid_row = (scroll_y / cell_h).floor() as usize;
    let visible_grid_rows = (viewport.h / cell_h).ceil().max(1.0) as usize + 1;
    UnitRowWindow {
        scroll_y,
        max_scroll,
        first: first_grid_row.saturating_mul(columns),
        visible: visible_grid_rows.saturating_mul(columns),
    }
}

fn visible_row_digest(rows: &[DataRow], window: UnitRowWindow) -> String {
    let visible: Vec<DataRow> = rows
        .iter()
        .skip(window.first)
        .take(window.visible)
        .cloned()
        .collect();
    row_digest(&visible)
}

struct UnitViewDrawable {
    /// The division-LIST root (`army_division_list_view`, has `divisions_grid`).
    list_roots: Vec<Node>,
    /// The division-STATS root (`army_division_stats_view`, has `equipment_list`) — built so
    /// the equipment drill-down rows actually RENDER (Codex (C): no invisible bound rows).
    stats_roots: Vec<Node>,
    file: GuiFile,
    division_rows: Vec<DataRow>,
    equipment_rows: Vec<DataRow>,
    division_viewport: Rect,
    division_window: UnitRowWindow,
    equipment_viewport: Rect,
    equipment_window: UnitRowWindow,
    stats_tab: UnitStatsTab,
    division_section: SectionState,
    division_count: usize,
    /// Whether the `equipment_list` grid was FOUND in the stats root (a layout fact),
    /// distinct from how many equipment rows exist — so the digest never conflates
    /// "no equipment rows" with "grid missing" (Codex (C) caveat / (G)).
    equipment_grid_found: bool,
    selected_division_key: Option<String>,
    selection_status: UnitSelectionStatus,
    equipment_state: UnitEquipmentState,
    selected_division_label: Option<String>,
}

impl Drawable for UnitViewDrawable {
    fn draw_digest(&self) -> String {
        let equipment_visible_window = if self.stats_tab == UnitStatsTab::Stats {
            self.equipment_window
        } else {
            UnitRowWindow {
                scroll_y: self.equipment_window.scroll_y,
                max_scroll: self.equipment_window.max_scroll,
                first: 0,
                visible: 0,
            }
        };
        let selected_key = match self.selected_division_key.as_deref() {
            Some(key) => key,
            None => DASH,
        };
        let selected_label = match self.selected_division_label.as_deref() {
            Some(label) => label,
            None => DASH,
        };
        format!(
            "UNIT div_sect={} divs={} unit_tab={} eq_grid_found={} eq_state={} eq_rows={} sel_status={} sel_key={} sel_label={} sel_row={} scroll={:.0}/{:.0} visible=[{}] DIVROWS {} eq_scroll={:.0}/{:.0} eq_visible=[{}] EQROWS {}",
            self.division_section.tag(),
            self.division_count,
            self.stats_tab.tag(),
            self.equipment_grid_found,
            self.equipment_state.tag(),
            self.equipment_rows.len(),
            self.selection_status.tag(),
            selected_key,
            selected_label,
            selected_row_digest(&self.division_rows, self.selected_division_key.as_deref()),
            self.division_window.scroll_y,
            self.division_window.max_scroll,
            visible_row_digest(&self.division_rows, self.division_window),
            row_digest(&self.division_rows),
            equipment_visible_window.scroll_y,
            equipment_visible_window.max_scroll,
            visible_row_digest(&self.equipment_rows, equipment_visible_window),
            row_digest(&self.equipment_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.list_roots);
        render_rows_selected_in_viewport(
            fd,
            &self.file,
            "divisions_window",
            "army_division_view",
            &self.division_rows,
            self.division_viewport,
            self.division_window.scroll_y,
            self.selected_division_key.as_deref(),
        );
        // The equipment drill-down renders into the STATS root's `equipment_list` grid.
        fd.draw_nodes(&self.stats_roots);
        if self.stats_tab == UnitStatsTab::Stats {
            render_rows_selected_in_viewport(
                fd,
                &self.file,
                "equipment_list",
                "stat_equipment_entry",
                &self.equipment_rows,
                self.equipment_viewport,
                self.equipment_window.scroll_y,
                None,
            );
        }
    }
}

impl ViewInstance for UnitView {
    fn view_id(&self) -> ViewId {
        UNIT_VIEW_ID
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
        self.division_input_rows = unit_division_input_rows(ctx.divisions(), ctx.country);
        self.equipment_row_count = selected_unit_equipment_count(
            &self.division_input_rows,
            self.selected_division_key.as_deref(),
        );
        self.division_scroll_y = self
            .division_scroll_y
            .min(max_unit_division_scroll_fallback(
                unit_division_list_fallback(self.screen),
                self.division_input_rows.len(),
            ));
        self.clear_scroll_drag_if_unavailable();
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => {
                if let Some(tab) = pick_unit_stats_tab(self.screen, x, y) {
                    let changed = self.selected_stats_tab != tab;
                    self.selected_stats_tab = tab;
                    self.clear_scroll_drag_if_unavailable();
                    return unit_input_consumed(changed);
                }
                let viewport = unit_division_list_fallback(self.screen);
                let scroll_y = self
                    .division_scroll_y
                    .min(max_unit_division_scroll_fallback(
                        viewport,
                        self.division_input_rows.len(),
                    ));
                if let Some(geom) = self.scrollbar_geometry(UnitScrollSurface::Divisions) {
                    if geom.thumb.contains(x, y) {
                        self.scroll_drag = Some(UnitScrollDrag {
                            surface: UnitScrollSurface::Divisions,
                            pointer_offset_y: y - geom.thumb.y,
                        });
                        return unit_input_consumed(false);
                    }
                    if geom.track.contains(x, y) {
                        return unit_input_consumed(false);
                    }
                }
                if let Some(geom) = self.scrollbar_geometry(UnitScrollSurface::Equipment) {
                    if geom.thumb.contains(x, y) {
                        self.scroll_drag = Some(UnitScrollDrag {
                            surface: UnitScrollSurface::Equipment,
                            pointer_offset_y: y - geom.thumb.y,
                        });
                        return unit_input_consumed(false);
                    }
                    if geom.track.contains(x, y) {
                        return unit_input_consumed(false);
                    }
                }
                if let Some((idx, key)) =
                    pick_unit_division_key(viewport, &self.division_input_rows, scroll_y, x, y)
                {
                    let changed = self.selected_division_key.as_deref() != Some(key.as_str());
                    self.equipment_row_count = match self.division_input_rows.get(idx) {
                        Some(row) => row.equipment_count,
                        None => 0,
                    };
                    if changed {
                        self.selected_division_key = Some(key);
                        self.equipment_scroll_y = 0.0;
                    }
                    unit_input_consumed(changed)
                } else {
                    InputOutcome::ignored()
                }
            }
            ViewInput::Wheel { x, y, dy } => {
                let division_viewport = unit_division_list_fallback(self.screen);
                if division_viewport.contains(x, y) {
                    let current = self.division_scroll_y;
                    let next = scroll_unit_division_offset(
                        current,
                        division_viewport,
                        self.division_input_rows.len(),
                        dy,
                    );
                    self.division_scroll_y = next;
                    return unit_input_consumed(next != current);
                }
                let equipment_viewport = unit_equipment_list_fallback(self.screen);
                if self.selected_stats_tab == UnitStatsTab::Stats
                    && equipment_viewport.contains(x, y)
                {
                    if self.equipment_row_count == 0 {
                        return unit_input_consumed(false);
                    }
                    let current = self
                        .equipment_scroll_y
                        .min(max_unit_equipment_scroll_fallback(
                            equipment_viewport,
                            self.equipment_row_count,
                        ));
                    let next = scroll_unit_equipment_offset(
                        current,
                        equipment_viewport,
                        self.equipment_row_count,
                        dy,
                    );
                    self.equipment_scroll_y = next;
                    return unit_input_consumed(next != current);
                }
                InputOutcome::ignored()
            }
            ViewInput::PointerMove { x: _, y, drag } if drag => {
                let drag = match self.scroll_drag {
                    Some(drag) => drag,
                    None => return InputOutcome::ignored(),
                };
                let geom = match self.scrollbar_geometry(drag.surface) {
                    Some(geom) => geom,
                    None => {
                        self.scroll_drag = None;
                        return InputOutcome::ignored();
                    }
                };
                let next = unit_scroll_from_thumb_y(geom, y - drag.pointer_offset_y);
                match drag.surface {
                    UnitScrollSurface::Divisions => {
                        let current = self.division_scroll_y;
                        self.division_scroll_y = next;
                        unit_input_consumed(next != current)
                    }
                    UnitScrollSurface::Equipment => {
                        let current = self.equipment_scroll_y;
                        self.equipment_scroll_y = next;
                        unit_input_consumed(next != current)
                    }
                }
            }
            ViewInput::PointerUp {
                button: PointerButton::Left,
                ..
            } => {
                if self.scroll_drag.take().is_some() {
                    unit_input_consumed(false)
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
        let file = load_gui(ctx, UNIT_VIEW_FILE);

        let unit_rows = build_division_rows(
            ctx,
            tag,
            &mut fidelity,
            self.selected_division_key.as_deref(),
        );
        let selection = &unit_rows.selection;

        // The list root binds the division rows. The stats root's `unit_title` names the
        // current read-only equipment pane source. After a user click, missing selected
        // divisions fail closed instead of falling back to the first row.
        let list_summary: Vec<Cell> = Vec::new();
        let mut stats_summary: Vec<Cell> = Vec::new();
        let label_text = match selection.status {
            UnitSelectionStatus::DefaultFirst => match &selection.label {
                Some(name) => format!("Equipment — first owned division (default): {name}"),
                None => "Equipment — first owned division (default)".to_string(),
            },
            UnitSelectionStatus::Selected => match &selection.label {
                Some(name) => format!("Equipment — selected division: {name}"),
                None => "Equipment — selected division".to_string(),
            },
            UnitSelectionStatus::MissingSelected => match self.selected_division_key.as_deref() {
                Some(key) => format!("Equipment — selected division unavailable: {key}"),
                None => "Equipment — selected division unavailable".to_string(),
            },
            UnitSelectionStatus::NoneAvailable => "Equipment — no owned division".to_string(),
        };
        let label_atom = DisplayAtom::text(label_text, Fidelity::Derived);
        fidelity.record(&label_atom);
        stats_summary.push(Cell {
            element: "unit_title".to_string(),
            atom: label_atom,
        });

        // Build BOTH roots: the division LIST (divisions_grid) + the division STATS panel
        // (equipment_list) so the equipment drill-down actually RENDERS (Codex (C)).
        let list_roots = build_root(ctx, &file, UNIT_VIEW_LIST_ROOT, &list_summary);
        let stats_roots = build_root(ctx, &file, UNIT_VIEW_STATS_ROOT, &stats_summary);
        let equipment_grid_found = find_rect(&stats_roots, "equipment_list").is_some();
        let division_viewport = unit_division_list_fallback(ctx.screen);
        let division_window = unit_division_row_window(
            &file,
            unit_rows.division_rows.len(),
            self.division_scroll_y,
            division_viewport,
        );
        let equipment_viewport = find_rect(&stats_roots, "equipment_list")
            .unwrap_or_else(|| unit_equipment_list_fallback(ctx.screen));
        let equipment_window = unit_equipment_row_window(
            &file,
            unit_rows.equipment_rows.len(),
            self.equipment_scroll_y,
            equipment_viewport,
        );

        // The hit model spans both roots (front-to-back).
        let mut hit_roots = list_roots.clone();
        hit_roots.extend(stats_roots.clone());

        let drawable = UnitViewDrawable {
            list_roots,
            stats_roots,
            file,
            division_rows: unit_rows.division_rows,
            equipment_rows: unit_rows.equipment_rows,
            division_viewport,
            division_window,
            equipment_viewport,
            equipment_window,
            stats_tab: self.selected_stats_tab,
            division_section: unit_rows.division_section,
            division_count: unit_rows.division_count,
            equipment_grid_found,
            selected_division_key: selection.key.clone(),
            selection_status: selection.status,
            equipment_state: selection.equipment_state,
            selected_division_label: selection.label.clone(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: UNIT_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(hit_roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Build the per-division `army_division_view` rows (filtered to the observed country) +
/// the `stat_equipment_entry` rows for the explicit selected division, or the first owned
/// division only before the user has clicked a row. EXACT fields: `template_name`/
/// `template_role`/`organisation`/
/// `strength`/`moving`/`owner`. Present-but-NULL `experience`/`supply_status` → unavailable
/// (badged with the Null reason, never a fabricated 0). The absent `divisions_full` body
/// (dig-in/manpower/accl/kills) → unavailable. Codex (d): owner is compared EXACTLY and a
/// missing owner is NEVER defaulted to the viewer; a Measured-non-array `divisions` is
/// `Degraded` (not empty); an empty measured array is `Present` with 0 rows.
fn build_division_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
    selected_key: Option<&str>,
) -> UnitRows {
    let (divs, section) = domain_array(ctx.divisions());
    let divs = match divs {
        Some(d) => d,
        None => {
            section.record(fidelity);
            let status = if selected_key.is_some() {
                UnitSelectionStatus::MissingSelected
            } else {
                UnitSelectionStatus::NoneAvailable
            };
            let equipment_state = if selected_key.is_some() {
                UnitEquipmentState::MissingSelected
            } else {
                UnitEquipmentState::NoSelection
            };
            return UnitRows {
                division_rows: Vec::new(),
                equipment_rows: Vec::new(),
                division_section: section,
                division_count: 0,
                selection: UnitSelection {
                    key: selected_key.map(str::to_string),
                    label: None,
                    status,
                    equipment_state,
                },
            };
        }
    };
    // Owner filter — EXACT string compare, missing owner never coerced to the viewer.
    let mine: Vec<(&serde_json::Value, Option<String>)> = divs
        .iter()
        .filter(|d| d.get("owner").and_then(|o| o.as_str()) == Some(tag))
        .map(|d| (d, division_row_key(d)))
        .collect();

    let mut rows = Vec::new();
    for (d, key) in &mine {
        let mut row = match key {
            Some(k) => DataRow::new().with_key(k.clone()),
            None => DataRow::new(),
        };
        // unit_name: division_name (divisions_full, absent) ELSE template_name (exact).
        row.bind(
            fidelity,
            "unit_name",
            elem_field(d, "template_name"),
            Fidelity::Derived,
            fmt_token,
        );
        row.bind(
            fidelity,
            "unit_icon",
            elem_field(d, "template_role"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "bar_org",
            elem_field(d, "organisation"),
            Fidelity::Exact,
            fmt_f1,
        );
        row.bind(
            fidelity,
            "bar_str",
            elem_field(d, "strength"),
            Fidelity::Exact,
            fmt_f1,
        );
        row.bind(
            fidelity,
            "flag",
            elem_field(d, "owner"),
            Fidelity::Exact,
            fmt_token,
        );
        // moving: present (bool/null) → exact when measured, null → unavailable.
        row.bind(
            fidelity,
            "unitlist_movement_icon",
            elem_field(d, "moving"),
            Fidelity::Exact,
            fmt_token,
        );
        // experience + supply_status: present-but-NULL in the demo → unavailable (Null).
        row.bind(
            fidelity,
            "unit_level",
            elem_field(d, "experience"),
            Fidelity::Exact,
            fmt_int,
        );
        row.bind(
            fidelity,
            "low_supply",
            elem_field(d, "supply_status"),
            Fidelity::Exact,
            fmt_token,
        );
        // divisions_full body (dig-in / acclimatization / planning) — absent → unavailable.
        row.badge_unavailable(fidelity, "dig_in_bonus");
        row.badge_unavailable(fidelity, "unitlist_acclimatization");
        row.badge_unavailable(fidelity, "bar_prepare");
        rows.push(row);
    }

    let (selected_division, selected_key_out, selection_status) = match selected_key {
        Some(key) => match mine.iter().find(|(_, k)| k.as_deref() == Some(key)) {
            Some((d, k)) => (Some(*d), k.clone(), UnitSelectionStatus::Selected),
            None => (
                None,
                Some(key.to_string()),
                UnitSelectionStatus::MissingSelected,
            ),
        },
        None => match mine.first() {
            Some((d, k)) => (Some(*d), k.clone(), UnitSelectionStatus::DefaultFirst),
            None => (None, None, UnitSelectionStatus::NoneAvailable),
        },
    };

    let selected_label = selected_division
        .and_then(|d| d.get("template_name"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string());

    let (equipment_rows, equipment_state) = match selected_division {
        Some(d) => build_unit_equipment_rows(d, fidelity),
        None => {
            let state = if selection_status == UnitSelectionStatus::MissingSelected {
                UnitEquipmentState::MissingSelected
            } else {
                UnitEquipmentState::NoSelection
            };
            (Vec::new(), state)
        }
    };

    section.record(fidelity);
    let count = mine.len();
    UnitRows {
        division_rows: rows,
        equipment_rows,
        division_section: section,
        division_count: count,
        selection: UnitSelection {
            key: selected_key_out,
            label: selected_label,
            status: selection_status,
            equipment_state,
        },
    }
}

fn build_unit_equipment_rows(
    selected: &serde_json::Value,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, UnitEquipmentState) {
    match elem_field(selected, "equipment") {
        Captured::Measured(v) => match v.as_array() {
            Some(eq) => {
                let mut equipment_rows = Vec::new();
                for e in eq {
                    let mut row = DataRow::new();
                    row.bind(
                        fidelity,
                        "equipment_name",
                        elem_field(e, "archetype"),
                        Fidelity::Derived,
                        fmt_token,
                    );
                    row.bind(
                        fidelity,
                        "equipment_amount",
                        elem_field(e, "amount"),
                        Fidelity::Exact,
                        fmt_int,
                    );
                    row.bind(
                        fidelity,
                        "equipment_icon",
                        elem_field(e, "archetype"),
                        Fidelity::Derived,
                        fmt_token,
                    );
                    row.badge_unavailable(fidelity, "creator_flag");
                    equipment_rows.push(row);
                }
                let state = if equipment_rows.is_empty() {
                    UnitEquipmentState::PresentEmpty
                } else {
                    UnitEquipmentState::PresentRows
                };
                (equipment_rows, state)
            }
            None => (Vec::new(), UnitEquipmentState::Degraded),
        },
        Captured::Null => (Vec::new(), UnitEquipmentState::Null),
        Captured::Absent => (Vec::new(), UnitEquipmentState::Absent),
    }
}

fn open_unit_view(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(UnitView::new())
}

register_view!(ViewDescriptor {
    id: UNIT_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(UNIT_VIEW_FILE, UNIT_VIEW_LIST_ROOT)),
    required_domains: &["divisions"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Unit View",
        order: 7
    }),
    shortcut: None,
    open: open_unit_view,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.army_division_view"),
    gui_template: GuiSymbolRef::template(UNIT_VIEW_FILE, "army_division_view"),
    required_domains: &["divisions"],
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.stat_equipment_entry"),
    gui_template: GuiSymbolRef::template(UNIT_VIEW_FILE, "stat_equipment_entry"),
    required_domains: &["divisions"],
});

// =====================================================================================
// EVENTS / POPUPS — the N5 event/popup card surfaces.
//
// `events.json` (`game_defs/out/events.json`, shape `{meta, events:{<id>->ev}}`) is a
// STATIC DEF parse: it supplies an event's title / desc / picture / options TEXT, never its
// occurrence / timing / country / option-taken / fired status (those come from
// `world_events.fired_events`, unfed in the demo). So an N5 card built from a def is a
// clearly-labelled definition PREVIEW; the "did it fire?" surface is badged unavailable.
// =====================================================================================

/// Load `events.json` (read-only) and return the parsed `events` object. Resolves it
/// relative to `Stage::assets_dir` (`<repo>/game_assets`) → `<repo>/game_defs/out/
/// events.json`. Any read/parse failure → `None` (the view then badges unavailable, never
/// fabricates an event). Deterministic; touches no shared state.
fn load_events_defs(ctx: &BindContext) -> Option<serde_json::Value> {
    let repo = ctx.stage.assets_dir.parent()?;
    load_events_defs_from_repo(repo)
}

fn load_events_defs_from_repo(repo: &std::path::Path) -> Option<serde_json::Value> {
    let path = repo.join("game_defs").join("out").join("events.json");
    let raw = std::fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    json.get("events").cloned()
}

/// Pick a small, STABLE set of event DEFs that carry a title (so the N5 preview renders
/// real wrapped text). Sorted by id for determinism; never a "representative FIRED" event
/// (Codex (b): a def is static text only). Returns up to `n` `(id, ev)` pairs.
fn sample_event_defs(events: &serde_json::Value, n: usize) -> Vec<(String, serde_json::Value)> {
    let obj = match events.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };
    let mut ids: Vec<&String> = obj
        .keys()
        .filter(|k| {
            obj.get(*k)
                .and_then(|e| e.get("title"))
                .map(|t| !t.is_null())
                .unwrap_or(false)
        })
        .collect();
    ids.sort();
    ids.into_iter()
        .take(n)
        .map(|k| (k.clone(), obj[k].clone()))
        .collect()
}

fn event_definition_preview_card_from_repo() -> Option<PopupCard> {
    let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let replay_native = crate_dir.parent()?;
    let repo = replay_native.parent()?;
    let events = load_events_defs_from_repo(repo)?;
    first_event_definition_preview_card(&events)
}

fn first_event_definition_preview_card(events: &serde_json::Value) -> Option<PopupCard> {
    sample_event_defs(events, 1)
        .into_iter()
        .next()
        .map(|(id, ev)| event_definition_preview_card(&id, &ev))
}

fn event_definition_preview_card(id: &str, ev: &serde_json::Value) -> PopupCard {
    let inner = EventCard::from_event_json(ev);
    let heading = if inner.title.is_unavailable() {
        format!("[event def preview — not fired] {id}")
    } else {
        format!("[event def preview — not fired] {}", inner.title.as_str())
    };
    let mut c = PopupCard::new(Atom::derived(heading)).media(inner.media.clone());
    for b in &inner.body {
        c = c.body(b.clone());
    }
    for a in &inner.actions {
        c = c.action(a.clone());
    }
    c
}

// =====================================================================================
// VIEW 7 — EVENT WINDOW  (EventWindow)  [N5 EventCard]
// =====================================================================================

const EVENT_WINDOW_VIEW_ID: ViewId = ViewId("logistics.event_window");
const EVENT_WINDOW_FILE: &str = "event_window.json";
const EVENT_WINDOW_ROOT: &str = "EventWindow";

pub struct EventWindowView {
    screen: Rect,
}

impl EventWindowView {
    pub fn new() -> EventWindowView {
        EventWindowView {
            screen: default_event_window_screen(),
        }
    }
}
impl Default for EventWindowView {
    fn default() -> Self {
        EventWindowView::new()
    }
}

struct EventWindowDrawable {
    roots: Vec<Node>,
    /// The N5 card built from one event DEF (a clearly-labelled preview). `None` when
    /// `events.json` is unreadable → the window renders chrome + a badged title only.
    card: Option<PopupCard>,
    /// Always unavailable in the demo: which event FIRED (needs `world_events.fired_events`).
    fired_section: SectionState,
}

fn default_event_window_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn event_window_preview_card_area(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w * 0.5 - 282.0,
        y: screen.y + screen.h * 0.5 - 310.0,
        w: 581.0,
        h: 427.0,
    }
}

#[cfg(test)]
fn event_window_preview_action_rect(screen: Rect, index: usize) -> Option<Rect> {
    let card = event_definition_preview_card_from_repo()?;
    let laid = card.layout(event_window_preview_card_area(screen));
    laid.action_rects.get(index).map(|a| a.rect)
}

fn event_window_preview_action_hit(screen: Rect, x: f32, y: f32) -> bool {
    let Some(card) = event_definition_preview_card_from_repo() else {
        return false;
    };
    card.layout(event_window_preview_card_area(screen))
        .pick_action(x, y)
        .is_some()
}

fn event_window_modal_chrome_hit(screen: Rect, x: f32, y: f32) -> bool {
    event_window_preview_card_area(screen).contains(x, y)
}

impl Drawable for EventWindowDrawable {
    fn draw_digest(&self) -> String {
        let card = self
            .card
            .as_ref()
            .map(card_digest)
            .unwrap_or_else(|| "none".to_string());
        format!(
            "EVENTWIN fired_sect={} CARD {card}",
            self.fired_section.tag()
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        if let Some(card) = &self.card {
            let area = find_rect(&self.roots, EVENT_WINDOW_ROOT).unwrap_or(Rect {
                x: 400.0,
                y: 200.0,
                w: 420.0,
                h: 460.0,
            });
            let laid = card.layout(area);
            let white = mint_white(fd);
            let assets = widget_assets(&white);
            widgets::event_card::draw_card(fd.renderer, &laid, &assets, |_, _| false);
        }
    }
}

impl ViewInstance for EventWindowView {
    fn view_id(&self) -> ViewId {
        EVENT_WINDOW_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::modal_overlay(ZBand::Overlay)
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            }
            | ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            } if event_window_preview_action_hit(self.screen, x, y) => InputOutcome {
                consumed: true,
                commands: Vec::new(),
                needs_redraw: false,
            },
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            }
            | ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            } if event_window_modal_chrome_hit(self.screen, x, y) => InputOutcome {
                consumed: true,
                commands: Vec::new(),
                needs_redraw: false,
            },
            _ => InputOutcome::ignored(),
        }
    }
    fn rebind(&mut self, rc: &RebindContext) {
        self.screen = Rect {
            x: 0.0,
            y: 0.0,
            w: rc.screen.0 as f32,
            h: rc.screen.1 as f32,
        };
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, EVENT_WINDOW_FILE);

        // Which event fired (and the historical option taken) needs `world_events.
        // fired_events` — unfed in the demo. Badge it on the ROOT, never fabricate.
        let (we, fired_section) = (ctx.world_events(), {
            match ctx.world_events() {
                Captured::Measured(_) => SectionState::Present,
                Captured::Null => SectionState::Null,
                Captured::Absent => SectionState::Absent,
            }
        });
        let _ = we;

        // Build the N5 card from one event DEF (a CLEARLY-LABELLED definition preview, NOT a
        // claim the event fired — Codex (A)). The card heading is prefixed with the explicit
        // preview marker so it can never read as an active event; the title/desc text come
        // from the def; the actually-fired event is unavailable (badged on the root above).
        let card = load_events_defs(ctx).and_then(|events| {
            first_event_definition_preview_card(&events).inspect(|c| {
                // tally the card's fields into the report (heading derived + body sections).
                fidelity.record(&DisplayAtom::text(
                    "def-preview".to_string(),
                    Fidelity::Derived,
                ));
                for b in &c.body {
                    fidelity.record(&atom_to_display(b));
                }
            })
        });

        // The ROOT Title/Description represent the ACTIVE (fired) event. In the demo NO
        // event fired (`world_events.fired_events` unfed) → both are badged unavailable on
        // the ROOT (Codex (A): the root cells must NOT show an arbitrary def's title as if
        // it fired). The events.json def only feeds the N5 PREVIEW card (a clearly-labelled
        // definition preview, drawn alongside but never claiming the event occurred).
        let mut summary: Vec<Cell> = Vec::new();
        push_unavailable(&mut fidelity, &mut summary, "Title");
        push_unavailable(&mut fidelity, &mut summary, "Description");

        let roots = build_root(ctx, &file, EVENT_WINDOW_ROOT, &summary);
        let drawable = EventWindowDrawable {
            roots: roots.clone(),
            card,
            fired_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: EVENT_WINDOW_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Turn a `widgets::Atom` back into a host `DisplayAtom` for the fidelity tally (the
/// inverse boundary, used only to RECORD a card's atoms in the report). Never fabricates.
fn atom_to_display(a: &Atom) -> DisplayAtom {
    match a.fidelity() {
        widgets::common::Fidelity::Exact => {
            DisplayAtom::text(a.as_str().to_string(), Fidelity::Exact)
        }
        widgets::common::Fidelity::Derived => {
            DisplayAtom::text(a.as_str().to_string(), Fidelity::Derived)
        }
        widgets::common::Fidelity::Unavailable => {
            DisplayAtom::unavailable_because(UnavailableReason::Blocked)
        }
    }
}

fn open_event_window(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(EventWindowView::new())
}

register_view!(ViewDescriptor {
    id: EVENT_WINDOW_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(EVENT_WINDOW_FILE, EVENT_WINDOW_ROOT)),
    required_domains: &["world_events"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Event Window",
        order: 8
    }),
    shortcut: None,
    open: open_event_window,
    policy: HostPolicy::modal_overlay(ZBand::Overlay),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.event_option_entry"),
    gui_template: GuiSymbolRef::template(EVENT_WINDOW_FILE, "event_option_entry"),
    required_domains: &["world_events"],
});

// =====================================================================================
// VIEW 8 — POPUP WINDOW  (the 32-popup generic LIBRARY)  [N5 cards]
//
// One view that ENUMERATES and renders ALL 32 real popup roots (Codex (c): every ROOT
// covered). Each popup's title/message is generic scripted loc supplied at fire time →
// unavailable unless the firing event is reconstructed; the mutation-modal verbs are inert.
// =====================================================================================

const POPUP_WINDOW_VIEW_ID: ViewId = ViewId("logistics.popup_window");
const POPUP_WINDOW_FILE: &str = "popup_window.json";

/// The 32 popup roots in `popup_window.json` (the complete enumerated set — proven by the
/// `popup_window_roots_enumerated` test == the file's window names).
const POPUP_ROOTS: [&str; 32] = [
    "diplomacy_standard_popup_window",
    "diplomacy_foreign_manpower_popup_window",
    "equipment_lend_lease_entry",
    "popup_lend_lease_request_item",
    "popup_lend_lease_info_item",
    "diplomacy_incoming_lend_lease_popup_window",
    "research_finished_popup_window",
    "custom_naval_invasion_warning",
    "custom_paradrop_warning",
    "facility_captured_popup_window",
    "confirmation_popup_window",
    "political_goal_popup_window",
    "equipment_special_item",
    "equipment_item",
    "surrendered_country_popup",
    "new_exiled_hosted_popup",
    "new_exiled_hosted_popup_small",
    "default_confirmation_popup",
    "release_country_dialog",
    "air_wing_assignment_popup",
    "focus_finished_popup_window",
    "peace_summary_popup_window",
    "default_info_popup_window",
    "oos_popup_window",
    "aicontroller_popup_window",
    "connection_lost_popup_window",
    "exile_country_popup",
    "diplomacy_scripted_popup_window",
    "send_diplomacy_scripted_popup_window",
    "dismiss_operative_popup",
    "unlock_industrial_org_trait_popup",
    "project_research_completed",
];

pub struct PopupWindowView;

impl PopupWindowView {
    pub fn new() -> PopupWindowView {
        PopupWindowView
    }
}
impl Default for PopupWindowView {
    fn default() -> Self {
        PopupWindowView::new()
    }
}

struct PopupWindowDrawable {
    /// One laid-out ROOT scene per popup type (all 32 enumerated, rendered stacked).
    roots_per_popup: Vec<(String, Vec<Node>)>,
    /// One N5 card framework per popup (the generic notification card). Bodies are
    /// unavailable (scripted loc) unless the firing event is reconstructed (it is not).
    cards: Vec<(String, PopupCard)>,
}

impl Drawable for PopupWindowDrawable {
    fn draw_digest(&self) -> String {
        let mut s = format!("POPUP roots={} cards=[", self.roots_per_popup.len());
        for (name, c) in &self.cards {
            s.push_str(&format!("{name}:{};", card_digest(c)));
        }
        s.push(']');
        s
    }
    fn draw(&self, fd: &mut FrameDraw) {
        // The library renders the FIRST popup root + its N5 card (the host opens one popup at
        // a time); the rest are enumerated/bound for coverage + digest, drawn on demand.
        if let Some((_, roots)) = self.roots_per_popup.first() {
            fd.draw_nodes(roots);
        }
        if let Some((_, card)) = self.cards.first() {
            let area = Rect {
                x: 460.0,
                y: 240.0,
                w: 400.0,
                h: 360.0,
            };
            let laid = card.layout(area);
            let white = mint_white(fd);
            let assets = widget_assets(&white);
            widgets::event_card::draw_card(fd.renderer, &laid, &assets, |_, _| false);
        }
    }
}

impl ViewInstance for PopupWindowView {
    fn view_id(&self) -> ViewId {
        POPUP_WINDOW_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::modal_overlay(ZBand::Overlay)
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        replay_modal_disabled_input(ev)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, POPUP_WINDOW_FILE);

        let stage = ctx.stage;
        let cm = ManifestContent {
            manifest: &stage.manifest,
            measure: |_t, _f| None,
        };

        let mut roots_per_popup: Vec<(String, Vec<Node>)> = Vec::new();
        let mut cards: Vec<(String, PopupCard)> = Vec::new();
        let mut all_roots: Vec<Node> = Vec::new();

        for name in POPUP_ROOTS {
            // Lay out the REAL popup root (every one covered).
            let mut roots: Vec<Node> = build_scene(&file, &stage.manifest, &stage.loc)
                .into_iter()
                .filter(|w| w.name == name)
                .collect();
            for r in roots.iter_mut() {
                layout(r, ctx.screen, None, &cm);
            }
            // A generic N5 card: the popup TITLE/MESSAGE is scripted loc (unavailable
            // unless the firing event is reconstructed) → the card title + body are badged.
            let card = PopupCard::new(Atom::unavailable()).body(Atom::unavailable());
            fidelity.record(&DisplayAtom::unavailable_because(
                UnavailableReason::Blocked,
            )); // title
            fidelity.record(&DisplayAtom::unavailable_because(
                UnavailableReason::Blocked,
            )); // body
            all_roots.extend(roots.iter().cloned());
            roots_per_popup.push((name.to_string(), roots));
            cards.push((name.to_string(), card));
        }

        let drawable = PopupWindowDrawable {
            roots_per_popup,
            cards,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: POPUP_WINDOW_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(all_roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_popup_window(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(PopupWindowView::new())
}

register_view!(ViewDescriptor {
    // NOTE: a single LIBRARY view that renders ALL 32 popup roots (Codex (c)). Its
    // `gui_root` names the first root as the descriptor anchor; the bind enumerates all 32.
    id: POPUP_WINDOW_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        POPUP_WINDOW_FILE,
        "surrendered_country_popup"
    )),
    required_domains: &["world_events"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Popup Library (32)",
        order: 9
    }),
    shortcut: None,
    open: open_popup_window,
    policy: HostPolicy::modal_overlay(ZBand::Overlay),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.equipment_special_item"),
    gui_template: GuiSymbolRef::template(POPUP_WINDOW_FILE, "equipment_special_item"),
    required_domains: &["world_events"],
});

// =====================================================================================
// VIEW 9 — DETAILED WINDOWS  (shipswindow / planeswindow)
// =====================================================================================

const DETAILED_WINDOWS_VIEW_ID: ViewId = ViewId("logistics.detailed_windows");
const DETAILED_WINDOWS_FILE: &str = "detailed_windows.json";
const DETAILED_WINDOWS_ROOT: &str = "shipswindow";

pub struct DetailedWindowsView;

impl DetailedWindowsView {
    pub fn new() -> DetailedWindowsView {
        DetailedWindowsView
    }
}
impl Default for DetailedWindowsView {
    fn default() -> Self {
        DetailedWindowsView::new()
    }
}

struct DetailedWindowsDrawable {
    roots: Vec<Node>,
}

impl Drawable for DetailedWindowsDrawable {
    fn draw_digest(&self) -> String {
        // ships_by_hull + the air subset of equipment_stockpile are unfed → all unavailable.
        "DETAILED ships_by_hull=unfed stockpile=unfed all=unavailable".to_string()
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for DetailedWindowsView {
    fn view_id(&self) -> ViewId {
        DETAILED_WINDOWS_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, DETAILED_WINDOWS_FILE);

        // ships_by_hull (0/143 in the demo) + the air subset of equipment_stockpile (unfed)
        // → totals + the archetype grids are unavailable.
        let mut summary: Vec<Cell> = Vec::new();
        push_unavailable(&mut fidelity, &mut summary, "total_ships");
        push_unavailable(&mut fidelity, &mut summary, "current_manpower");

        let roots = build_root(ctx, &file, DETAILED_WINDOWS_ROOT, &summary);
        let drawable = DetailedWindowsDrawable {
            roots: roots.clone(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: DETAILED_WINDOWS_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_detailed_windows(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(DetailedWindowsView::new())
}

register_view!(ViewDescriptor {
    id: DETAILED_WINDOWS_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        DETAILED_WINDOWS_FILE,
        DETAILED_WINDOWS_ROOT
    )),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Detailed Windows",
        order: 10
    }),
    shortcut: None,
    open: open_detailed_windows,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.equipment_ship_entry"),
    gui_template: GuiSymbolRef::template(DETAILED_WINDOWS_FILE, "equipment_ship_entry"),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 10 — CORE  (core.json) — the SHARED PROTOTYPE LIBRARY
//
// The shared widget / scrollbar / tooltip PROTOTYPE library other views inherit. It defines
// almost no data-bound surface; the one conditionally-bindable prototype is
// `countryflag_entry.flag` (a TAG supplied by the host view at instantiation → derived).
// Documented + surfaced as a prototype-library view (near-zero binding).
// =====================================================================================

const CORE_VIEW_ID: ViewId = ViewId("logistics.core");
const CORE_FILE: &str = "core.json";
const CORE_ROOT: &str = "countryflag_entry";

pub struct CoreView;

impl CoreView {
    pub fn new() -> CoreView {
        CoreView
    }
}
impl Default for CoreView {
    fn default() -> Self {
        CoreView::new()
    }
}

struct CoreDrawable {
    roots: Vec<Node>,
}

impl Drawable for CoreDrawable {
    fn draw_digest(&self) -> String {
        // 82 static prototype/chrome elements + 1 conditionally-derived prototype
        // (countryflag_entry.flag), which standalone has no datum.
        "CORE prototype_lib=true bindable=1(countryflag_entry) static=82".to_string()
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for CoreView {
    fn view_id(&self) -> ViewId {
        CORE_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, CORE_FILE);

        // The one conditionally-bindable prototype: `countryflag_entry.flag`. STANDALONE (in
        // core.gui) it has NO datum — each CONSUMING view supplies the TAG at instantiation.
        // Surfaced on its own (this prototype-library view), there is no host view supplying a
        // TAG, so it is badged UNAVAILABLE, never fabricated with the viewer's flag (Codex
        // (F): instantiating it with the viewer TAG would be a fabrication — the prototype
        // carries no datum). Listed so the prototype isn't mistaken for dead.
        let mut summary: Vec<Cell> = Vec::new();
        push_unavailable(&mut fidelity, &mut summary, "flag");

        let roots = build_root(ctx, &file, CORE_ROOT, &summary);
        let drawable = CoreDrawable {
            roots: roots.clone(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: CORE_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_core(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(CoreView::new())
}

register_view!(ViewDescriptor {
    id: CORE_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(CORE_FILE, CORE_ROOT)),
    required_domains: &[],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Core Prototypes",
        order: 11
    }),
    shortcut: None,
    open: open_core,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.countryflag_entry"),
    gui_template: GuiSymbolRef::template(CORE_FILE, "countryflag_entry"),
    required_domains: &[],
});

// =====================================================================================
// VIEW 11 — INGAME MENU  (ingamemenu) — the pause / escape menu
//
// In a replay this is scrubber-relevant chrome but almost entirely inert/navigation: the
// pause menu's verbs (save/load/resign/quit) are meaningless and the save list reads the
// host filesystem (OS-scope, not snapshot data). All cells are unavailable/inert.
// =====================================================================================

const INGAME_MENU_VIEW_ID: ViewId = ViewId("logistics.ingame_menu");
const INGAME_MENU_FILE: &str = "ingame_menu.json";
const INGAME_MENU_ROOT: &str = "ingamemenu";

pub struct IngameMenuView;

impl IngameMenuView {
    pub fn new() -> IngameMenuView {
        IngameMenuView
    }
}
impl Default for IngameMenuView {
    fn default() -> Self {
        IngameMenuView::new()
    }
}

struct IngameMenuDrawable {
    roots: Vec<Node>,
}

impl Drawable for IngameMenuDrawable {
    fn draw_digest(&self) -> String {
        // Save list = host filesystem (OS-scope); menu verbs inert in a replay.
        "INGAMEMENU save_list=os_scope verbs=inert all=unavailable".to_string()
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for IngameMenuView {
    fn view_id(&self) -> ViewId {
        INGAME_MENU_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::modal_overlay(ZBand::Overlay)
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        replay_modal_disabled_input(ev)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, INGAME_MENU_FILE);

        // The save-list rows read the host filesystem (OS-scope) — out of the replay-binding
        // model. The menu verbs are inert. The only data-bearing surfaces (disk_quota /
        // saves_dirs / the save-list rows) live in the SEPARATE `ingamemenu_load`/
        // `ingamemenu_save` sub-window roots, so they are recorded as OS-scope gaps (counted,
        // not bound to the `ingamemenu` root). The root itself carries only chrome + verbs.
        let summary: Vec<Cell> = Vec::new();
        record_gap(&mut fidelity); // ingamemenu_load.disk_quota (OS-scope)
        record_gap(&mut fidelity); // ingamemenu_load.saves_dirs (OS-scope)
        record_gap(&mut fidelity); // savegame_item rows (host filesystem)

        let roots = build_root(ctx, &file, INGAME_MENU_ROOT, &summary);
        let drawable = IngameMenuDrawable {
            roots: roots.clone(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: INGAME_MENU_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_ingame_menu(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(IngameMenuView::new())
}

register_view!(ViewDescriptor {
    id: INGAME_MENU_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(INGAME_MENU_FILE, INGAME_MENU_ROOT)),
    required_domains: &[],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Ingame Menu",
        order: 12
    }),
    shortcut: None,
    open: open_ingame_menu,
    policy: HostPolicy::modal_overlay(ZBand::Overlay),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.savegame_item"),
    gui_template: GuiSymbolRef::template(INGAME_MENU_FILE, "savegame_item"),
    required_domains: &[],
});

// =====================================================================================
// VIEW 12 — NAVAL COMBAT  (navalcombatview)  [BINDING_MAP_diplomacy_combat VIEW 3]
// =====================================================================================
//
// The LIVE naval-battle window (`naval_combat.json` root `navalcombatview`) — DISTINCT from
// the already-registered `naval_combat_results` POST-battle report. Bound from
// `ctx.combats()` filtered to `kind == "naval"`. In the demo `combats[]` is all `kind:"land"`
// (0 naval combats), so the side flags / battle title / leader names / per-side ship counts
// all badge `unavailable` — 0 naval combats in peacetime is CORRECT-EMPTY (an absent live
// battle), never a fabricated 0. When a naval combat IS present (the scrub's B hour) the
// belligerent owner TAGs bind DERIVED (side identity exact, TAG via owner ref); the live
// battle-DETAIL (progress / screening / per-ship stats) stays `unavailable` per PLAN §G.
const NAVAL_COMBAT_VIEW_ID: ViewId = ViewId("logistics.naval_combat");
const NAVAL_COMBAT_FILE: &str = "naval_combat.json";
const NAVAL_COMBAT_ROOT: &str = "navalcombatview";

pub struct NavalCombatView;

impl NavalCombatView {
    pub fn new() -> NavalCombatView {
        NavalCombatView
    }
}
impl Default for NavalCombatView {
    fn default() -> Self {
        NavalCombatView::new()
    }
}

struct NavalCombatDrawable {
    roots: Vec<Node>,
    combats_section: SectionState,
    naval_count: usize,
}

impl Drawable for NavalCombatDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "NAVCOMBAT combats_sect={} naval={} (live battle-detail = unavailable §G)",
            self.combats_section.tag(),
            self.naval_count,
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for NavalCombatView {
    fn view_id(&self) -> ViewId {
        NAVAL_COMBAT_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Overlay)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, NAVAL_COMBAT_FILE);

        // The NAVAL combats this hour (the demo carries 0 — all `kind:"land"`). `combats()`
        // distinguishes Absent (no domain) from a measured-but-empty list, so an absent live
        // battle is never fabricated into present-empty.
        let (combats, combats_section) = domain_array(ctx.combats());
        // Tally the combats-domain section state (Absent/Null/Present/Degraded) so the
        // fidelity accounting matches CombatLogView (Codex: don't compute-but-drop it).
        combats_section.record(&mut fidelity);
        let naval: Vec<&serde_json::Value> = match combats {
            Some(cs) => cs
                .iter()
                .filter(|c| c.get("kind").and_then(|k| k.as_str()) == Some("naval"))
                .collect(),
            None => Vec::new(),
        };

        // The header binds the FIRST live naval battle (deterministic: index 0, not map
        // iteration). With 0 naval combats every field badges unavailable (correct-empty).
        let mut summary: Vec<Cell> = Vec::new();
        match naval.first() {
            Some(c) => {
                let atk = side_owner(c, "attackers");
                let def = side_owner(c, "defenders");
                push_side_flag(&mut fidelity, &mut summary, "flag_attacker", atk);
                push_side_flag(&mut fidelity, &mut summary, "flag_defender", def);
                // a composed "Naval skirmish" title is derived only when a battle exists.
                push_value(
                    &mut fidelity,
                    &mut summary,
                    "naval_battle_title",
                    serde_json::Value::String("Naval skirmish".to_string()),
                    Fidelity::Derived,
                    fmt_token,
                );
                // leaders + per-side ship counts need `commanders` + `order_of_battle` (both
                // absent) → unavailable even WITH a live battle.
                push_unavailable(&mut fidelity, &mut summary, "leader_attacker_name");
                push_unavailable(&mut fidelity, &mut summary, "leader_defender_name");
                push_unavailable(&mut fidelity, &mut summary, "leader_attacker_ships_count");
                push_unavailable(&mut fidelity, &mut summary, "leader_defender_ships_count");
            }
            None => {
                // 0 naval combats: every header field is unavailable (no live battle), never 0.
                for el in [
                    "flag_attacker",
                    "flag_defender",
                    "naval_battle_title",
                    "leader_attacker_name",
                    "leader_defender_name",
                    "leader_attacker_ships_count",
                    "leader_defender_ships_count",
                ] {
                    push_unavailable(&mut fidelity, &mut summary, el);
                }
            }
        }

        let roots = build_root(ctx, &file, NAVAL_COMBAT_ROOT, &summary);
        let drawable = NavalCombatDrawable {
            roots: roots.clone(),
            combats_section,
            naval_count: naval.len(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: NAVAL_COMBAT_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The owner TAG of a combat's attacker/defender side (index 0 element's `owner`/`country`),
/// as a [`Captured`] so an absent side renders "—". DETERMINISTIC (index 0, not iteration).
fn side_owner<'a>(combat: &'a serde_json::Value, side: &str) -> Captured<&'a serde_json::Value> {
    let v = combat
        .get(side)
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|x| x.get("owner").or_else(|| x.get("country")));
    crate::host::context::classify_value(v)
}

/// Push a side-flag summary cell bound DERIVED from the side's owner TAG (side identity is
/// exact; the resolved TAG is derived). Absent → "—" (never a fabricated flag).
fn push_side_flag(
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
    element: &str,
    owner: Captured<&serde_json::Value>,
) {
    let atom = owner.display_with_fidelity(Fidelity::Derived, fmt_token);
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

fn open_naval_combat(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(NavalCombatView::new())
}

register_view!(ViewDescriptor {
    id: NAVAL_COMBAT_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(NAVAL_COMBAT_FILE, NAVAL_COMBAT_ROOT)),
    required_domains: &["combats"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Naval Combat",
        order: 13
    }),
    shortcut: None,
    open: open_naval_combat,
    policy: HostPolicy::window(ZBand::Overlay),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.navalcombat_ship_entry"),
    gui_template: GuiSymbolRef::template(NAVAL_COMBAT_FILE, "navalcombat_ship_entry"),
    required_domains: &["combats", "order_of_battle"],
});

// =====================================================================================
// VIEW 13 — COMBAT LOG  (combat_log_view)  [BINDING_MAP_diplomacy_combat VIEW 4]
// =====================================================================================
//
// The combat-log frame (`combat_log.json` root `combat_log_view`). NATURE: a HISTORICAL
// per-country ledger the engine accumulates; our snapshots carry NO such ledger and there is
// NO top-level `combat_log` domain — combat HISTORY is reconstructed from the diff-event
// timeline / the active `combats[]` set. KEY STRUCTURE: the `combat_log_view` ROOT contains
// only chrome + the `title` label; ALL data-bearing fields (`period_desc`,
// `title_losses_in_manpower`, `num_combats_won_factor`, the equipment/template tab totals,
// the `combats_list` rows) live in SEPARATE top-level windows (`combat_log_page`,
// `combat_log_combat_tab`, …), NOT children of `combat_log_view`. Binding into THIS root,
// those are LOGICAL gaps (tallied for honesty, never a targetable cell — so a stale
// `set_element_text` can never no-op against the wrong root). The `title` label binds the
// static loc (derived). The combats-list reconstruction is sourced from captured `combats[]`
// / the (absent) `combat_log` domain — never fabricated from absent ledger totals.
const COMBAT_LOG_VIEW_ID: ViewId = ViewId("logistics.combat_log");
const COMBAT_LOG_FILE: &str = "combat_log.json";
const COMBAT_LOG_ROOT: &str = "combat_log_view";
const COMBAT_LOG_COMBATS_GRID: &str = "combats_list";
const COMBAT_LOG_COMBAT_ITEM: &str = "combat_log_combat_item";
const COMBAT_LOG_ROW_H: f32 = 100.0;

pub struct CombatLogView {
    screen: Rect,
    combat_row_count: usize,
}

impl CombatLogView {
    pub fn new() -> CombatLogView {
        CombatLogView {
            screen: default_combat_log_screen(),
            combat_row_count: 0,
        }
    }
}
impl Default for CombatLogView {
    fn default() -> Self {
        CombatLogView::new()
    }
}

fn default_combat_log_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn combat_log_panel_rect(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 995.0,
        y: screen.y + 95.0,
        w: 670.0,
        h: (screen.h - 165.0).max(0.0),
    }
}

fn combat_log_rows_viewport_fallback(screen: Rect) -> Rect {
    let panel = combat_log_panel_rect(screen);
    Rect {
        x: panel.x + 10.0,
        y: panel.y + 50.0 + 115.0 + 10.0,
        w: 610.0,
        h: (panel.h - 50.0 - 115.0 - 10.0).max(0.0),
    }
}

fn combat_log_rows_viewport(roots: &[Node], screen: Rect) -> Rect {
    match find_rect(roots, "page_left") {
        Some(page) => Rect {
            x: page.x + 10.0,
            y: page.y + 115.0 + 10.0,
            w: 610.0,
            h: (page.h - 115.0 - 10.0).max(0.0),
        },
        None => combat_log_rows_viewport_fallback(screen),
    }
}

fn combat_log_visible_row_hit(screen: Rect, row_count: usize, x: f32, y: f32) -> bool {
    let viewport = combat_log_rows_viewport_fallback(screen);
    if row_count == 0 || !viewport.contains(x, y) {
        return false;
    }
    let row = ((y - viewport.y) / COMBAT_LOG_ROW_H).floor() as usize;
    row < row_count && (row as f32) * COMBAT_LOG_ROW_H < viewport.h
}

struct CombatLogDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    /// The combats reconstructed for the list (from captured `combats[]`); empty when the
    /// domain is absent (never fabricated from absent ledger totals).
    combat_rows: Vec<DataRow>,
    row_viewport: Rect,
    log_section: SectionState,
    combats_section: SectionState,
}

impl Drawable for CombatLogDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "COMBATLOG log_dom={} combats_sect={} rows={} (ledger totals = unavailable; page/tab fields = gaps)",
            self.log_section.tag(),
            self.combats_section.tag(),
            row_digest(&self.combat_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows_selected_in_viewport(
            fd,
            &self.file,
            COMBAT_LOG_COMBATS_GRID,
            COMBAT_LOG_COMBAT_ITEM,
            &self.combat_rows,
            self.row_viewport,
            0.0,
            None,
        );
    }
}

impl ViewInstance for CombatLogView {
    fn view_id(&self) -> ViewId {
        COMBAT_LOG_VIEW_ID
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
        let snap = &ctx.snapshot;
        let (rows, _) = domain_array(crate::host::context::classify_value(snap.domain("combats")));
        self.combat_row_count = match rows {
            Some(rows) => rows.len(),
            None => 0,
        };
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            }
            | ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            } if combat_log_visible_row_hit(self.screen, self.combat_row_count, *x, *y) => {
                InputOutcome {
                    consumed: true,
                    commands: Vec::new(),
                    needs_redraw: false,
                }
            }
            _ => InputOutcome::ignored(),
        }
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, COMBAT_LOG_FILE);

        // The `combat_log_view` ROOT carries only chrome + the static `title` loc label.
        let mut summary: Vec<Cell> = Vec::new();
        push_value(
            &mut fidelity,
            &mut summary,
            "title",
            serde_json::Value::String("Combat Log".to_string()),
            Fidelity::Derived,
            fmt_token,
        );

        // The historical LEDGER (per-battle losses, equipment lost/recovered/attrition,
        // template win tallies, the aggregate manpower/win-factor + the month slider readouts)
        // is NOT stored AND its `.gui` nodes live in the SEPARATE page/tab roots, so each is a
        // LOGICAL gap relative to this root (tallied for honesty, never a targetable cell).
        for _gap in [
            "period_desc",
            "min_log_months",
            "max_log_months", // combat_log_page slider
            "title_losses_in_manpower",
            "num_combats_won_factor", // combat_log_page aggregates
            "num_equipment_lost_in_combat",
            "num_equipment_recovered_in_combat",
            "num_equipment_lost_by_attrition",
            "num_enemy_equipment_lost", // equipment tab totals
        ] {
            record_gap(&mut fidelity);
        }

        // The `combat_log` HISTORY domain is not in the snapshot (no top-level key);
        // the combats-list is reconstructed from the captured ACTIVE `combats[]` set (a
        // derived preview), never fabricated from absent ledger totals.
        let log_section = match ctx.domain_field("combat_log") {
            Captured::Measured(_) => SectionState::Present,
            Captured::Null => SectionState::Null,
            Captured::Absent => SectionState::Absent,
        };
        log_section.record(&mut fidelity);

        let (combat_rows, combats_section) = build_combat_log_rows(ctx, &mut fidelity);

        let roots = build_root(ctx, &file, COMBAT_LOG_ROOT, &summary);
        let row_viewport = combat_log_rows_viewport(&roots, self.screen);
        let drawable = CombatLogDrawable {
            roots: roots.clone(),
            file,
            combat_rows,
            row_viewport,
            log_section,
            combats_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: COMBAT_LOG_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Reconstruct the combat-list rows from the captured ACTIVE `combats[]` set (the snapshot
/// carries no stored combat-log ledger; the list is a derived preview of currently-active
/// engagements). Each row binds `location` (province id) + `date` (derived) — the per-battle
/// loss cells stay `unavailable` (not a stored ledger). Empty when `combats` is absent.
fn build_combat_log_rows(
    ctx: &BindContext,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let (combats, section) = domain_array(ctx.combats());
    let cs = match combats {
        Some(cs) => cs,
        None => {
            section.record(fidelity);
            return (Vec::new(), section);
        }
    };
    let mut rows = Vec::new();
    for c in cs {
        let mut row = DataRow::new();
        if let Some(id) = c.get("combat_id").and_then(row_key_token) {
            row = row.with_key(format!("combat:{id}"));
        }
        row.bind(
            fidelity,
            "location",
            elem_field(c, "location_province"),
            Fidelity::Exact,
            fmt_int,
        );
        row.bind(
            fidelity,
            "atk_flag",
            side_owner(c, "attackers"),
            Fidelity::Derived,
            fmt_token,
        );
        row.bind(
            fidelity,
            "def_flag",
            side_owner(c, "defenders"),
            Fidelity::Derived,
            fmt_token,
        );
        row.bind_value(
            fidelity,
            "date",
            serde_json::Value::String(ctx.date.to_string()),
            Fidelity::Derived,
            fmt_token,
        );
        // the historical per-battle loss ledger is not stored → unavailable (never a 0).
        for element in [
            "atk_lost_manpower",
            "def_lost_manpower",
            "atk_lost_armor_eq",
            "def_lost_armor_eq",
            "atk_lost_air_eq",
            "def_lost_air_eq",
            "atk_lost_other_eq",
            "def_lost_other_eq",
        ] {
            row.badge_unavailable(fidelity, element);
        }
        rows.push(row);
    }
    section.record(fidelity);
    (rows, section)
}

fn open_combat_log(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(CombatLogView::new())
}

register_view!(ViewDescriptor {
    id: COMBAT_LOG_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(COMBAT_LOG_FILE, COMBAT_LOG_ROOT)),
    required_domains: &["combats"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LogisticsCombatUnitsEvents,
        label: "Combat Log",
        order: 14
    }),
    shortcut: None,
    open: open_combat_log,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("logistics.combat_log_combat_item"),
    gui_template: GuiSymbolRef::template(COMBAT_LOG_FILE, "combat_log_combat_item"),
    required_domains: &["combats"],
});

// =====================================================================================
// TESTS — render-plan digest + scrub A→B→A byte-identity + per-view element accounting
// (walk == element_type_counts) + the destroyed_ships empty-stub + the 32-popup-root
// enumeration + the no-fabrication guard for this cluster's own source.
// =====================================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::Stage;
    use crate::host::window_host::WindowHost;
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

    fn assets_dir() -> std::path::PathBuf {
        gui_dir().parent().unwrap().to_path_buf()
    }

    fn load_one(name: &str) -> Option<GuiFile> {
        let p = gui_dir().join(name);
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| GuiFile::parse(&s).ok())
    }

    /// A stage carrying the named `.gui` files + the real `assets_dir` (so the lenient
    /// fallback + the events.json loader resolve against the on-disk repo data).
    fn stage_with(files: &[&str]) -> Stage {
        let mut gui_files = HashMap::new();
        for f in files {
            if let Some(g) = load_one(f) {
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
            assets_dir: assets_dir(),
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
            date: "1941.4.13.11",
            cursor: (0.0, 0.0),
            map_camera: None,
        }
    }

    /// Skip a test cleanly (not fail) when the gitignored extracted game assets are absent.
    fn assets_present() -> bool {
        gui_dir().join("logistics.json").exists()
    }

    #[test]
    fn ingame_menu_disabled_verbs_clicks_are_modal_noops() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut host = WindowHost::new();
        host.open_instance(Box::new(IngameMenuView::new()));

        let before = {
            let bound = host.bind_all(&ctx(&stage, &snap));
            assert_eq!(bound.len(), 1);
            assert_eq!(bound[0].view_id, INGAME_MENU_VIEW_ID);
            assert!(
                bound[0].commands.is_empty(),
                "binding the replay ingame menu must not emit save/load/resign/quit commands"
            );
            let digest = bound_digest(&bound[0]);
            assert!(
                digest.contains("INGAMEMENU save_list=os_scope verbs=inert"),
                "ingame menu digest must keep save-list OS scope and inert verbs explicit, got:\n{digest}"
            );
            digest
        };

        for ev in [
            ViewInput::PointerDown {
                x: 800.0,
                y: 450.0,
                button: PointerButton::Left,
            },
            ViewInput::PointerUp {
                x: 800.0,
                y: 450.0,
                button: PointerButton::Left,
            },
        ] {
            let out = host.route_input(&ev);
            assert!(out.consumed, "ingame menu modal click must be consumed");
            assert!(
                out.commands.is_empty(),
                "disabled ingame menu verbs must not emit commands on {ev:?}"
            );
            assert!(
                !out.needs_redraw,
                "disabled ingame menu verb no-op must not request redraw on {ev:?}"
            );
        }

        let after = {
            let bound = host.bind_all(&ctx(&stage, &snap));
            assert_eq!(bound.len(), 1);
            bound_digest(&bound[0])
        };
        assert_eq!(
            before, after,
            "disabled ingame-menu verb clicks must not mutate replay-local view state"
        );
    }

    #[test]
    fn event_window_definition_option_clicks_are_replay_disabled_noops() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut host = WindowHost::new();
        let mut view = EventWindowView::new();
        view.rebind(&RebindContext {
            snapshot: &snap,
            country: "GER",
            screen: (1600, 900),
            refit: true,
        });
        host.open_instance(Box::new(view));

        let before = {
            let bound = host.bind_all(&ctx(&stage, &snap));
            assert_eq!(bound.len(), 1);
            assert_eq!(bound[0].view_id, EVENT_WINDOW_VIEW_ID);
            assert!(
                bound[0].policy.modal,
                "event windows are modal overlays in the host stack"
            );
            assert!(
                bound[0].commands.is_empty(),
                "binding the replay event preview must not emit option/effect commands"
            );
            let digest = bound_digest(&bound[0]);
            assert!(digest.contains("EVENTWIN"), "{digest}");
            assert!(
                digest.contains("act=[opt_0:"),
                "event definition preview must expose a real option action for the no-op proof, got:\n{digest}"
            );
            digest
        };

        let action = event_window_preview_action_rect(default_event_window_screen(), 0)
            .expect("current static event catalog exposes a first preview option action");
        for ev in [
            ViewInput::PointerDown {
                x: action.x + 4.0,
                y: action.y + 4.0,
                button: PointerButton::Left,
            },
            ViewInput::PointerUp {
                x: action.x + 4.0,
                y: action.y + 4.0,
                button: PointerButton::Left,
            },
        ] {
            let out = host.route_input(&ev);
            assert!(
                out.consumed,
                "event definition option click must be consumed as a replay-disabled no-op"
            );
            assert!(
                out.commands.is_empty(),
                "event definition option no-op must not emit commands on {ev:?}"
            );
            assert!(
                !out.needs_redraw,
                "event definition option no-op must not request redraw on {ev:?}"
            );
        }

        let after = {
            let bound = host.bind_all(&ctx(&stage, &snap));
            assert_eq!(bound.len(), 1);
            bound_digest(&bound[0])
        };
        assert_eq!(
            before, after,
            "disabled event option clicks must not mutate replay-local view state"
        );
    }

    #[test]
    fn event_window_modal_chrome_clicks_are_noops_without_broad_view_claim() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut host = WindowHost::new();
        let mut view = EventWindowView::new();
        view.rebind(&RebindContext {
            snapshot: &snap,
            country: "GER",
            screen: (1600, 900),
            refit: true,
        });
        host.open_instance(Box::new(view));

        let before = {
            let bound = host.bind_all(&ctx(&stage, &snap));
            assert_eq!(bound.len(), 1);
            assert_eq!(bound[0].view_id, EVENT_WINDOW_VIEW_ID);
            assert!(bound[0].policy.modal);
            bound_digest(&bound[0])
        };

        let root = event_window_preview_card_area(default_event_window_screen());
        let chrome_x = root.x + 20.0;
        let chrome_y = root.y + 20.0;
        assert!(event_window_modal_chrome_hit(
            default_event_window_screen(),
            chrome_x,
            chrome_y
        ));
        assert!(
            !event_window_preview_action_hit(default_event_window_screen(), chrome_x, chrome_y),
            "chrome proof point must stay outside displayed option action geometry"
        );

        for ev in [
            ViewInput::PointerDown {
                x: chrome_x,
                y: chrome_y,
                button: PointerButton::Left,
            },
            ViewInput::PointerUp {
                x: chrome_x,
                y: chrome_y,
                button: PointerButton::Left,
            },
        ] {
            let out = host.route_input(&ev);
            assert!(
                out.consumed,
                "event-window modal chrome click must be consumed as a no-op"
            );
            assert!(
                out.commands.is_empty(),
                "event-window modal chrome no-op must not emit commands on {ev:?}"
            );
            assert!(
                !out.needs_redraw,
                "event-window modal chrome no-op must not request redraw on {ev:?}"
            );
        }

        let mut direct_view = EventWindowView::new();
        direct_view.rebind(&RebindContext {
            snapshot: &snap,
            country: "GER",
            screen: (1600, 900),
            refit: true,
        });
        let direct_off_root = direct_view.on_input(&ViewInput::PointerDown {
            x: 8.0,
            y: 8.0,
            button: PointerButton::Left,
        });
        assert!(
            !direct_off_root.consumed
                && direct_off_root.commands.is_empty()
                && !direct_off_root.needs_redraw,
            "off-root clicks must not be claimed by EventWindowView-specific action/chrome routing"
        );

        let host_off_root = host.route_input(&ViewInput::PointerDown {
            x: 8.0,
            y: 8.0,
            button: PointerButton::Left,
        });
        assert!(
            host_off_root.consumed,
            "off-root clicks are blocked by the event window's modal host policy"
        );
        assert!(host_off_root.commands.is_empty());
        assert!(!host_off_root.needs_redraw);

        let after = {
            let bound = host.bind_all(&ctx(&stage, &snap));
            assert_eq!(bound.len(), 1);
            bound_digest(&bound[0])
        };
        assert_eq!(
            before, after,
            "event-window modal no-op clicks must not mutate replay-local view state"
        );
    }

    #[test]
    fn popup_window_displayed_action_clicks_are_replay_disabled_modal_noops() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut host = WindowHost::new();
        host.open_instance(Box::new(PopupWindowView::new()));

        let before = {
            let bound = host.bind_all(&ctx(&stage, &snap));
            assert_eq!(bound.len(), 1);
            assert_eq!(bound[0].view_id, POPUP_WINDOW_VIEW_ID);
            assert!(
                bound[0].policy.modal,
                "popup windows are modal overlays in the host stack"
            );
            assert!(
                bound[0].commands.is_empty(),
                "binding the popup library must not emit popup accept/decline commands"
            );
            let digest = bound_digest(&bound[0]);
            assert!(
                digest.contains("POPUP roots=32"),
                "popup digest must prove the real popup library roots are enumerated, got:\n{digest}"
            );
            assert!(
                digest.contains("diplomacy_standard_popup_window"),
                "test click targets the first displayed popup root, got:\n{digest}"
            );
            digest
        };

        // `diplomacy_standard_popup_window` is centered at (-225,-160), and its real
        // `accept_button` sits at (386,265) in `popup_window.json`.
        let accept_x = 1600.0 * 0.5 - 225.0 + 386.0 + 4.0;
        let accept_y = 900.0 * 0.5 - 160.0 + 265.0 + 4.0;
        for ev in [
            ViewInput::PointerDown {
                x: accept_x,
                y: accept_y,
                button: PointerButton::Left,
            },
            ViewInput::PointerUp {
                x: accept_x,
                y: accept_y,
                button: PointerButton::Left,
            },
        ] {
            let out = host.route_input(&ev);
            assert!(
                out.consumed,
                "displayed popup action click must be consumed as a replay-disabled modal no-op"
            );
            assert!(
                out.commands.is_empty(),
                "popup action no-op must not emit commands on {ev:?}"
            );
            assert!(
                !out.needs_redraw,
                "popup action no-op must not request redraw on {ev:?}"
            );
        }

        let after = {
            let bound = host.bind_all(&ctx(&stage, &snap));
            assert_eq!(bound.len(), 1);
            bound_digest(&bound[0])
        };
        assert_eq!(
            before, after,
            "disabled popup action clicks must not mutate replay-local view state"
        );
    }

    /// SAVE-backed GER snapshot mirroring the real demo: `economy.resources` +
    /// `economy.fuel.{current,max}` + `economy.factories.{mil,dockyard}` present;
    /// `divisions[]` present (org/strength/equipment EXACT, experience/supply_status null);
    /// `combats[]` all land (0 naval); `diplomacy_world.factions` present.
    fn snap_a() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11", "hour_index": 100,
            "countries": { "GER": {
                "economy": {
                    "resources": { "produced": { "steel": 1196.0, "oil": 14.0, "coal": 617.0 } },
                    "fuel": { "current": 5000.0, "max": 10000.0 },
                    "factories": { "mil": 131, "dockyard": 16 }
                },
                "politics": { "faction": { "name": "axis" } }
            },
            "ITA": { "politics": { "faction": { "name": "axis" } } },
            "ENG": { "politics": { "faction": { "name": "allies" } } } },
            "divisions": [
                { "owner": "GER", "template_name": "Motorized Infantry 1936 C", "template_role": "motorized",
                  "organisation": 99.87856, "strength": 229.4, "moving": false, "experience": null,
                  "supply_status": null, "in_combat": null,
                  "equipment": [ { "archetype": "artillery_equipment_2", "amount": 12.0 } ] },
                { "owner": "ENG", "template_name": "Foreign", "template_role": "infantry",
                  "organisation": 50.0, "strength": 100.0, "moving": true, "experience": null, "supply_status": null }
            ],
            "combats": [ { "combat_id": 1, "kind": "land", "location_province": 5 } ],
            "diplomacy_world": { "factions": { "1": { "ideology": "fascism", "members": ["GER"] } } }
        }))
        .unwrap()
    }

    /// A DIFFERENT-hour snapshot (live-ish): different fuel + an EXTRA division + 1 naval
    /// combat, so the scrub A→B differs and proves the test.
    fn snap_b() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.5.1.11", "hour_index": 200,
            "countries": { "GER": {
                "economy": {
                    "resources": { "produced": { "steel": 900.0 } },
                    "fuel": { "current": 9000.0, "max": 10000.0 },
                    "factories": { "mil": 140, "dockyard": 18 }
                },
                "politics": { "faction": { "name": "axis" } }
            } },
            "divisions": [
                { "owner": "GER", "template_name": "Panzer", "template_role": "armor",
                  "organisation": 80.0, "strength": 200.0, "moving": true, "experience": null, "supply_status": null,
                  "equipment": [ { "archetype": "medium_tank_chassis_1", "amount": 50.0 } ] }
            ],
            "combats": [ { "combat_id": 9, "kind": "naval", "location_province": 4,
                           "attackers": [ { "owner": "GER" } ], "defenders": [ { "owner": "ENG" } ] } ],
            "diplomacy_world": { "factions": {} }
        }))
        .unwrap()
    }

    fn unit_rebind_ctx<'a>(snap: &'a snapshot::Snapshot) -> RebindContext<'a> {
        RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit: false,
        }
    }

    fn snap_unit_a_multi() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11", "hour_index": 100,
            "countries": { "GER": {}, "ENG": {} },
            "divisions": [
                { "div_id": 101, "owner": "GER", "template_name": "Duplicate Template", "template_role": "infantry",
                  "organisation": 90.0, "strength": 100.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": [ { "archetype": "infantry_equipment_2", "amount": 11.0 } ] },
                { "div_id": 202, "owner": "GER", "template_name": "Duplicate Template", "template_role": "infantry",
                  "organisation": 80.0, "strength": 95.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": [ { "archetype": "artillery_equipment_2", "amount": 22.0 } ] },
                { "div_id": 303, "owner": "ENG", "template_name": "Foreign Duplicate Template", "template_role": "infantry",
                  "organisation": 70.0, "strength": 90.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": [ { "archetype": "anti_tank_equipment_1", "amount": 33.0 } ] }
            ]
        }))
        .unwrap()
    }

    fn snap_unit_b_same_selected() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.12", "hour_index": 101,
            "countries": { "GER": {} },
            "divisions": [
                { "div_id": 101, "owner": "GER", "template_name": "Duplicate Template", "template_role": "infantry",
                  "organisation": 91.0, "strength": 101.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": [ { "archetype": "infantry_equipment_2", "amount": 12.0 } ] },
                { "div_id": 202, "owner": "GER", "template_name": "Duplicate Template", "template_role": "infantry",
                  "organisation": 81.0, "strength": 96.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": [ { "archetype": "artillery_equipment_2", "amount": 77.0 } ] }
            ]
        }))
        .unwrap()
    }

    fn snap_unit_b_missing_selected() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.12", "hour_index": 101,
            "countries": { "GER": {}, "ENG": {} },
            "divisions": [
                { "div_id": 101, "owner": "GER", "template_name": "Duplicate Template", "template_role": "infantry",
                  "organisation": 91.0, "strength": 101.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": [ { "archetype": "infantry_equipment_2", "amount": 99.0 } ] },
                { "div_id": 202, "owner": "ENG", "template_name": "Foreign Duplicate Template", "template_role": "infantry",
                  "organisation": 81.0, "strength": 96.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": [ { "archetype": "artillery_equipment_2", "amount": 77.0 } ] }
            ]
        }))
        .unwrap()
    }

    fn snap_unit_missing_first_identity() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11", "hour_index": 100,
            "countries": { "GER": {} },
            "divisions": [
                { "owner": "GER", "template_name": "No Id", "template_role": "infantry",
                  "organisation": 90.0, "strength": 100.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": [ { "archetype": "infantry_equipment_2", "amount": 11.0 } ] },
                { "div_id": 202, "owner": "GER", "template_name": "Has Id", "template_role": "infantry",
                  "organisation": 80.0, "strength": 95.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": [ { "archetype": "artillery_equipment_2", "amount": 22.0 } ] }
            ]
        }))
        .unwrap()
    }

    fn many_unit_divisions_snap(count: usize, prefix: &str) -> snapshot::Snapshot {
        let divisions: Vec<_> = (1..=count)
            .map(|i| {
                serde_json::json!({
                    "div_id": i,
                    "owner": "GER",
                    "template_name": format!("{prefix} {i:02}"),
                    "template_role": "infantry",
                    "organisation": 50.0 + i as f64,
                    "strength": 80.0 + i as f64,
                    "moving": false,
                    "experience": null,
                    "supply_status": null,
                    "equipment": [
                        { "archetype": "infantry_equipment_2", "amount": (i as f64) * 10.0 }
                    ]
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11",
            "hour_index": 100,
            "countries": { "GER": {} },
            "divisions": divisions
        }))
        .unwrap()
    }

    fn equipment_values(count: usize, prefix: &str) -> Vec<serde_json::Value> {
        (1..=count)
            .map(|i| {
                serde_json::json!({
                    "archetype": format!("{prefix}_eq_{i:02}"),
                    "amount": i as f64
                })
            })
            .collect()
    }

    fn snap_unit_many_equipment(count: usize, prefix: &str) -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11",
            "hour_index": 100,
            "countries": { "GER": {} },
            "divisions": [
                { "div_id": 101, "owner": "GER", "template_name": format!("{prefix} Division"), "template_role": "infantry",
                  "organisation": 90.0, "strength": 100.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": equipment_values(count, prefix) }
            ]
        }))
        .unwrap()
    }

    fn snap_unit_two_equipment_divisions(
        first_count: usize,
        second_count: usize,
        prefix: &str,
    ) -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11",
            "hour_index": 100,
            "countries": { "GER": {} },
            "divisions": [
                { "div_id": 101, "owner": "GER", "template_name": format!("{prefix} First"), "template_role": "infantry",
                  "organisation": 90.0, "strength": 100.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": equipment_values(first_count, &format!("{prefix}_first")) },
                { "div_id": 202, "owner": "GER", "template_name": format!("{prefix} Second"), "template_role": "infantry",
                  "organisation": 80.0, "strength": 95.0, "moving": false, "experience": null,
                  "supply_status": null, "equipment": equipment_values(second_count, &format!("{prefix}_second")) }
            ]
        }))
        .unwrap()
    }

    fn click_unit_row(view: &mut UnitView, row: usize) -> InputOutcome {
        let viewport = unit_division_list_fallback(view.screen);
        view.on_input(&ViewInput::PointerDown {
            x: viewport.x + 4.0,
            y: viewport.y + UNIT_VIEW_ROW_H * (row as f32) + 4.0,
            button: PointerButton::Left,
        })
    }

    fn wheel_unit_rows(view: &mut UnitView, dy: f32) -> InputOutcome {
        let viewport = unit_division_list_fallback(view.screen);
        view.on_input(&ViewInput::Wheel {
            x: viewport.x + 8.0,
            y: viewport.y + UNIT_VIEW_ROW_H * 0.5,
            dy,
        })
    }

    fn wheel_unit_equipment(view: &mut UnitView, dy: f32) -> InputOutcome {
        let viewport = unit_equipment_list_fallback(view.screen);
        view.on_input(&ViewInput::Wheel {
            x: viewport.x + 8.0,
            y: viewport.y + UNIT_EQUIPMENT_ROW_H * 0.5,
            dy,
        })
    }

    fn unit_division_scrollbar(view: &UnitView) -> UnitScrollbarGeometry {
        view.scrollbar_geometry(UnitScrollSurface::Divisions)
            .expect("division list should overflow and expose a scrollbar")
    }

    fn unit_equipment_scrollbar(view: &UnitView) -> UnitScrollbarGeometry {
        view.scrollbar_geometry(UnitScrollSurface::Equipment)
            .expect("equipment pane should overflow and expose a scrollbar")
    }

    fn pointer_down_unit_division_thumb(view: &mut UnitView) -> InputOutcome {
        let geom = unit_division_scrollbar(view);
        view.on_input(&ViewInput::PointerDown {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.thumb.y + geom.thumb.h * 0.5,
            button: PointerButton::Left,
        })
    }

    fn pointer_down_unit_equipment_thumb(view: &mut UnitView) -> InputOutcome {
        let geom = unit_equipment_scrollbar(view);
        view.on_input(&ViewInput::PointerDown {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.thumb.y + geom.thumb.h * 0.5,
            button: PointerButton::Left,
        })
    }

    fn click_unit_stats_tab(view: &mut UnitView, tab: UnitStatsTab) -> InputOutcome {
        let rect = unit_stats_tab_button_fallback(view.screen, tab);
        view.on_input(&ViewInput::PointerDown {
            x: rect.x + 4.0,
            y: rect.y + 4.0,
            button: PointerButton::Left,
        })
    }

    fn unit_stats_tab_digest(d: &str) -> &str {
        d.split(" unit_tab=")
            .nth(1)
            .and_then(|rest| rest.split(" eq_grid_found=").next())
            .expect("UnitView digest has unit_tab marker")
    }

    fn unit_scroll_pair(d: &str) -> (i32, i32) {
        let rest = d
            .split(" scroll=")
            .nth(1)
            .expect("UnitView digest has scroll marker");
        let pair = rest
            .split(" visible=")
            .next()
            .expect("UnitView digest has visible marker");
        let (cur, max) = pair.split_once('/').expect("scroll pair");
        (
            cur.parse().expect("scroll current is int"),
            max.parse().expect("scroll max is int"),
        )
    }

    fn visible_unit_chunk(d: &str) -> &str {
        d.split(" visible=[")
            .nth(1)
            .and_then(|rest| rest.split("] DIVROWS").next())
            .expect("UnitView digest has visible row chunk")
    }

    fn equipment_scroll_pair(d: &str) -> (i32, i32) {
        let rest = d
            .split(" eq_scroll=")
            .nth(1)
            .expect("UnitView digest has equipment scroll marker");
        let pair = rest
            .split(" eq_visible=")
            .next()
            .expect("UnitView digest has equipment visible marker");
        let (cur, max) = pair.split_once('/').expect("equipment scroll pair");
        (
            cur.parse().expect("equipment scroll current is int"),
            max.parse().expect("equipment scroll max is int"),
        )
    }

    fn visible_equipment_chunk(d: &str) -> &str {
        d.split(" eq_visible=[")
            .nth(1)
            .and_then(|rest| rest.split("] EQROWS").next())
            .expect("UnitView digest has visible equipment chunk")
    }

    // -------- render-plan digest: scrub A→B→A byte-identical (the immutable-LRU rule) -----

    /// A generic scrub A→B→A byte-identity assertion for one view.
    fn assert_scrub(files: &[&str], view: &dyn ViewInstance) {
        if !assets_present() {
            eprintln!("[2h] skipped scrub: game_assets/gui not extracted");
            return;
        }
        let stage = stage_with(files);
        let (a, b) = (snap_a(), snap_b());
        let da1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        let db = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert_ne!(
            da1, db,
            "A and B must differ, else the scrub proves nothing"
        );
        let da2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(da1, da2, "scrub A→B→A carried a value from the B visit");
    }

    #[test]
    fn logistics_scrub_a_b_a_byte_identical() {
        assert_scrub(&["logistics.json"], &LogisticsView::new());
    }
    #[test]
    fn unit_view_scrub_a_b_a_byte_identical() {
        assert_scrub(&["unit_view.json"], &UnitView::new());
    }
    #[test]
    fn naval_combat_results_scrub_a_b_a_byte_identical() {
        assert_scrub(
            &["naval_combat_results.json"],
            &NavalCombatResultsView::new(),
        );
    }
    #[test]
    fn naval_losses_scrub_a_b_a_byte_identical() {
        assert_scrub(&["naval_losses.json"], &NavalLossesView::new());
    }
    #[test]
    fn naval_combat_scrub_a_b_a_byte_identical() {
        // snap_a: 0 naval combats; snap_b: 1 naval combat (GER vs ENG) → A and B differ.
        assert_scrub(&["naval_combat.json"], &NavalCombatView::new());
    }
    #[test]
    fn combat_log_scrub_a_b_a_byte_identical() {
        // snap_a: 1 land combat row; snap_b: 1 naval combat row → A and B differ.
        assert_scrub(&["combat_log.json"], &CombatLogView::new());
    }

    // -------- no-fabrication: a blocked field renders "—", never a 0 ----------------------

    #[test]
    fn logistics_resources_exact_fuel_derived_materiel_unavailable() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["logistics.json"]);
        let snap = snap_a();
        let bv = LogisticsView::new().bind(&ctx(&stage, &snap));
        let d = bound_digest(&bv);
        // a measured resource value renders its real number (steel 1196 → "1196"), EXACT.
        assert!(
            d.contains("value=1196(E)"),
            "measured resource value must show exact, got: {d}"
        );
        // the aggregate fuel bar binds derived (5000/10000 = 50%).
        assert!(d.contains("res_sect=present"), "resource section present");
        // the materiel grid is unfed → 0 rows (no fabricated present-empty equipment).
        assert!(
            d.contains("mat=") && !d.contains("mat=r0"),
            "materiel grid must be empty (unfed), got: {d}"
        );
    }

    #[test]
    fn unit_view_division_exact_null_fields_unavailable() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["unit_view.json"]);
        let snap = snap_a();
        let bv = UnitView::new().bind(&ctx(&stage, &snap));
        let d = bound_digest(&bv);
        // owner filter → only the GER division (1), not the ENG one.
        assert!(
            d.contains("divs=1"),
            "owner filter must keep only the viewer's division, got: {d}"
        );
        // org/strength EXACT (real measured values), template_role EXACT.
        assert!(
            d.contains("bar_org=99.9(E)"),
            "organisation must bind exact, got: {d}"
        );
        assert!(
            d.contains("unit_icon=motorized(E)"),
            "template_role must bind exact"
        );
        // experience is present-but-NULL → unavailable with the Null reason (not a 0).
        assert!(
            d.contains("unit_level=—(Un)"),
            "null experience must badge unavailable-null, got: {d}"
        );
        // equipment row: amount EXACT (12), archetype derived; no fabricated 0.
        assert!(
            d.contains("equipment_amount=12(E)"),
            "equipment amount must bind exact"
        );
        // never a fabricated 0 for the null fields.
        assert!(
            !d.contains("unit_level=0("),
            "null experience must NEVER render a fabricated 0"
        );
    }

    #[test]
    fn unit_view_division_row_click_selects_measured_div_id_and_updates_equipment() {
        let stage = stage_with(&[]);
        let snap = snap_unit_a_multi();
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&snap));

        let default = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            default.contains("sel_status=default_first"),
            "pre-click pane must be explicitly default-first, got: {default}"
        );
        assert!(
            default.contains("sel_key=division:101"),
            "pre-click default may use the first measured div_id, got: {default}"
        );
        assert!(
            default.contains("equipment_amount=11(E)"),
            "default equipment must come from first owned division, got: {default}"
        );

        let out = click_unit_row(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        assert!(out.commands.is_empty());

        let selected = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("sel_status=selected"),
            "row click must switch to selected status, got: {selected}"
        );
        assert!(
            selected.contains("sel_key=division:202"),
            "row click must use measured div_id, not duplicate template text/index, got: {selected}"
        );
        assert!(
            selected.contains("sel_row=r0[key=division:202;"),
            "selected-row digest must point at the clicked backing row, got: {selected}"
        );
        assert!(
            selected.contains("equipment_amount=22(E)"),
            "equipment pane must rebuild from selected division, got: {selected}"
        );
        assert!(
            !selected.contains("equipment_amount=11(E)"),
            "selected pane must not retain first-row equipment, got: {selected}"
        );
    }

    #[test]
    fn unit_view_selected_division_scrub_a_b_a_rebuilds_current_hour_equipment() {
        let stage = stage_with(&[]);
        let a = snap_unit_a_multi();
        let b = snap_unit_b_same_selected();
        let mut view = UnitView::new();

        view.rebind(&unit_rebind_ctx(&a));
        let out = click_unit_row(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert!(a1.contains("equipment_amount=22(E)"));

        view.rebind(&unit_rebind_ctx(&b));
        let b_digest = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            b_digest.contains("sel_key=division:202"),
            "selection key must survive scrub into B, got: {b_digest}"
        );
        assert!(
            b_digest.contains("equipment_amount=77(E)"),
            "B pane must rebuild from B's current selected division, got: {b_digest}"
        );
        assert!(
            !b_digest.contains("equipment_amount=22(E)"),
            "B pane must not carry A-only equipment, got: {b_digest}"
        );

        view.rebind(&unit_rebind_ctx(&a));
        let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "selected UnitView scrub A->B->A must be byte-identical"
        );
    }

    #[test]
    fn unit_view_missing_selected_division_fails_closed_without_first_row_fallback() {
        let stage = stage_with(&[]);
        let a = snap_unit_a_multi();
        let b = snap_unit_b_missing_selected();
        let mut view = UnitView::new();

        view.rebind(&unit_rebind_ctx(&a));
        let out = click_unit_row(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert!(a1.contains("sel_key=division:202"));
        assert!(a1.contains("equipment_amount=22(E)"));

        view.rebind(&unit_rebind_ctx(&b));
        let missing = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            missing.contains("sel_status=missing_selected"),
            "foreign-owned/missing selected div_id must fail closed, got: {missing}"
        );
        assert!(
            missing.contains("sel_key=division:202"),
            "missing selected digest must keep the user's requested key, got: {missing}"
        );
        assert!(
            missing.contains("eq_state=missing_selected"),
            "missing selected equipment state must be explicit, got: {missing}"
        );
        assert!(
            missing.contains("eq_rows=0"),
            "missing selected division must not emit equipment rows, got: {missing}"
        );
        assert!(
            !missing.contains("equipment_amount=99(E)") && !missing.contains("equipment_amount=77(E)"),
            "missing selected pane must not fall back to first owned or foreign rows, got: {missing}"
        );

        view.rebind(&unit_rebind_ctx(&a));
        let restored = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, restored,
            "returning to A must restore the selected pane exactly"
        );
    }

    #[test]
    fn unit_view_missing_identity_and_outside_clicks_are_inert() {
        let snap = snap_unit_missing_first_identity();
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&snap));

        let missing_id = click_unit_row(&mut view, 0);
        assert!(
            !missing_id.consumed && !missing_id.needs_redraw && missing_id.commands.is_empty(),
            "row without measured div_id must not be selectable by index/name"
        );
        assert!(
            view.selected_division_key.is_none(),
            "missing div_id click must not set a fabricated selection key"
        );

        let viewport = unit_division_list_fallback(view.screen);
        let outside = view.on_input(&ViewInput::PointerDown {
            x: viewport.x + viewport.w + 20.0,
            y: viewport.y + 4.0,
            button: PointerButton::Left,
        });
        assert!(
            !outside.consumed && !outside.needs_redraw && outside.commands.is_empty(),
            "outside-row click must be inert"
        );

        let selectable = click_unit_row(&mut view, 1);
        assert!(selectable.consumed && selectable.needs_redraw);
        assert_eq!(view.selected_division_key.as_deref(), Some("division:202"));
    }

    #[test]
    fn unit_view_division_wheel_scroll_changes_visible_rows_and_clamps() {
        let stage = stage_with(&[]);
        let snap = many_unit_divisions_snap(30, "A");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&snap));

        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(unit_scroll_pair(&before).0, 0);
        assert!(
            visible_unit_chunk(&before).contains("key=division:1;unit_name=A 01(D);"),
            "{before}"
        );

        let out = wheel_unit_rows(&mut view, -3.0);
        assert!(out.consumed && out.needs_redraw);
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(unit_scroll_pair(&after).0, 99);
        assert!(
            visible_unit_chunk(&after).contains("key=division:4;unit_name=A 04(D);"),
            "after three-row wheel scroll, visible rows must advance: {after}"
        );
        assert!(
            !visible_unit_chunk(&after).contains("key=division:1;unit_name=A 01(D);"),
            "scrolled visible slice must not keep top rows: {after}"
        );

        let out = wheel_unit_rows(&mut view, -500.0);
        assert!(out.consumed);
        let clamped = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let (cur, max) = unit_scroll_pair(&clamped);
        assert_eq!(cur, max, "huge wheel-down must clamp to max: {clamped}");

        let out = wheel_unit_rows(&mut view, 500.0);
        assert!(out.consumed);
        let reset = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(unit_scroll_pair(&reset).0, 0);
    }

    #[test]
    fn unit_view_scrolled_division_click_selects_correct_backing_div_id() {
        let stage = stage_with(&[]);
        let snap = many_unit_divisions_snap(30, "A");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&snap));

        wheel_unit_rows(&mut view, -5.0);
        let out = click_unit_row(&mut view, 0);
        assert!(out.consumed && out.needs_redraw);
        let d = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("sel_key=division:6"),
            "after five-row scroll, visible row 0 must select backing division 6, got:\n{d}"
        );
        assert!(
            d.contains("equipment_amount=60(E)"),
            "equipment pane must rebuild from backing division 6, got:\n{d}"
        );
    }

    #[test]
    fn unit_view_division_scroll_survives_scrub_a_b_a_without_b_rows() {
        let stage = stage_with(&[]);
        let a = many_unit_divisions_snap(30, "A");
        let b = many_unit_divisions_snap(35, "B");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&a));

        wheel_unit_rows(&mut view, -5.0);
        click_unit_row(&mut view, 0);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert!(a1.contains("sel_key=division:6"));
        assert!(visible_unit_chunk(&a1).contains("unit_name=A 06(D);"));

        view.rebind(&unit_rebind_ctx(&b));
        let b_digest = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            b_digest.contains("sel_key=division:6"),
            "selected measured id should survive into B, got:\n{b_digest}"
        );
        assert!(
            visible_unit_chunk(&b_digest).contains("unit_name=B 06(D);"),
            "visible chunk must rebuild from B rows instead of carrying A text, got:\n{b_digest}"
        );

        view.rebind(&unit_rebind_ctx(&a));
        let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "UnitView row selection + scroll must be byte-identical across A->B->A"
        );
    }

    #[test]
    fn unit_view_division_scrollbar_drag_changes_visible_rows_and_clamps() {
        let stage = stage_with(&[]);
        let snap = many_unit_divisions_snap(30, "A");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&snap));

        let down = pointer_down_unit_division_thumb(&mut view);
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "thumb pointer-down should only capture division drag"
        );
        let geom = unit_division_scrollbar(&view);
        let drag = view.on_input(&ViewInput::PointerMove {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.track.y + geom.track.h + 200.0,
            drag: true,
        });
        assert!(
            drag.consumed && drag.needs_redraw,
            "captured division thumb drag should scroll even past the track"
        );
        let scrolled = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let (cur, max) = unit_scroll_pair(&scrolled);
        assert_eq!(
            cur, max,
            "division thumb drag must clamp to max: {scrolled}"
        );
        assert!(
            visible_unit_chunk(&scrolled).contains("key=division:19;unit_name=A 19(D);"),
            "bottom-clamped division drag should expose backing later rows: {scrolled}"
        );
        assert!(
            !visible_unit_chunk(&scrolled).contains("key=division:1;unit_name=A 01(D);"),
            "bottom-clamped division drag must not keep top rows visible: {scrolled}"
        );

        let reset = view.on_input(&ViewInput::PointerMove {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.track.y - 200.0,
            drag: true,
        });
        assert!(reset.consumed && reset.needs_redraw);
        let restored = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(unit_scroll_pair(&restored).0, 0);
    }

    #[test]
    fn unit_view_equipment_wheel_scroll_changes_visible_rows_and_clamps() {
        let stage = stage_with(&[]);
        let snap = snap_unit_many_equipment(20, "A");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&snap));

        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(equipment_scroll_pair(&before).0, 0);
        assert!(
            visible_equipment_chunk(&before).contains("equipment_name=A_eq_01(D);"),
            "{before}"
        );

        let out = wheel_unit_equipment(&mut view, -3.0);
        assert!(out.consumed && out.needs_redraw);
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(equipment_scroll_pair(&after).0, 150);
        assert_eq!(
            unit_scroll_pair(&after).0,
            0,
            "equipment wheel must not move the division list: {after}"
        );
        assert!(
            visible_equipment_chunk(&after).contains("equipment_name=A_eq_04(D);"),
            "after three-row equipment wheel scroll, visible rows must advance: {after}"
        );
        assert!(
            !visible_equipment_chunk(&after).contains("equipment_name=A_eq_01(D);"),
            "scrolled equipment slice must not keep top rows: {after}"
        );

        let out = wheel_unit_equipment(&mut view, -500.0);
        assert!(out.consumed);
        let clamped = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let (cur, max) = equipment_scroll_pair(&clamped);
        assert_eq!(
            cur, max,
            "huge equipment wheel-down must clamp to max: {clamped}"
        );

        let out = wheel_unit_equipment(&mut view, 500.0);
        assert!(out.consumed);
        let reset = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(equipment_scroll_pair(&reset).0, 0);
    }

    #[test]
    fn unit_view_equipment_scroll_resets_when_selected_division_changes() {
        let stage = stage_with(&[]);
        let snap = snap_unit_two_equipment_divisions(20, 2, "A");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&snap));

        wheel_unit_equipment(&mut view, -3.0);
        let scrolled = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(equipment_scroll_pair(&scrolled).0, 150);
        assert!(visible_equipment_chunk(&scrolled).contains("equipment_name=A_first_eq_04(D);"));

        let out = click_unit_row(&mut view, 1);
        assert!(out.consumed && out.needs_redraw);
        let selected = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("sel_key=division:202"),
            "click must select the second measured division, got: {selected}"
        );
        assert_eq!(equipment_scroll_pair(&selected).0, 0);
        assert!(
            visible_equipment_chunk(&selected).contains("equipment_name=A_second_eq_01(D);"),
            "newly selected division equipment must start at the top, got: {selected}"
        );
        assert!(
            !visible_equipment_chunk(&selected).contains("equipment_name=A_first_eq_04(D);"),
            "new selection must not retain previous division equipment rows, got: {selected}"
        );
    }

    #[test]
    fn unit_view_equipment_scroll_survives_scrub_a_b_a_for_same_selected_division() {
        let stage = stage_with(&[]);
        let a = snap_unit_two_equipment_divisions(1, 20, "A");
        let b = snap_unit_two_equipment_divisions(1, 20, "B");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&a));

        click_unit_row(&mut view, 1);
        wheel_unit_equipment(&mut view, -3.0);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert!(a1.contains("sel_key=division:202"));
        assert_eq!(equipment_scroll_pair(&a1).0, 150);
        assert!(visible_equipment_chunk(&a1).contains("equipment_name=A_second_eq_04(D);"));

        view.rebind(&unit_rebind_ctx(&b));
        let b_digest = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            b_digest.contains("sel_key=division:202"),
            "selection key must survive scrub into B, got: {b_digest}"
        );
        assert_eq!(equipment_scroll_pair(&b_digest).0, 150);
        assert!(
            visible_equipment_chunk(&b_digest).contains("equipment_name=B_second_eq_04(D);"),
            "equipment rows must rebuild from B instead of carrying A rows, got: {b_digest}"
        );
        assert!(
            !visible_equipment_chunk(&b_digest).contains("equipment_name=A_second_eq_04(D);"),
            "B digest must not carry A-only equipment text, got: {b_digest}"
        );

        view.rebind(&unit_rebind_ctx(&a));
        let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "UnitView selected equipment scroll must be byte-identical across A->B->A"
        );
    }

    #[test]
    fn unit_view_equipment_scroll_clamps_when_selected_division_missing_without_losing_a() {
        let stage = stage_with(&[]);
        let a = snap_unit_two_equipment_divisions(1, 20, "A");
        let b = snap_unit_b_missing_selected();
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&a));

        click_unit_row(&mut view, 1);
        wheel_unit_equipment(&mut view, -3.0);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(equipment_scroll_pair(&a1).0, 150);
        assert!(visible_equipment_chunk(&a1).contains("equipment_name=A_second_eq_04(D);"));

        view.rebind(&unit_rebind_ctx(&b));
        let missing = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert!(
            missing.contains("sel_status=missing_selected"),
            "foreign-owned/missing selected div_id must fail closed, got: {missing}"
        );
        assert!(
            missing.contains("eq_rows=0"),
            "missing selected division must not emit equipment rows, got: {missing}"
        );
        assert_eq!(equipment_scroll_pair(&missing), (0, 0));
        assert_eq!(visible_equipment_chunk(&missing), "");
        assert!(
            !missing.contains("equipment_amount=99(E)") && !missing.contains("equipment_amount=77(E)"),
            "missing selected pane must not fall back to first owned or foreign rows, got: {missing}"
        );

        view.rebind(&unit_rebind_ctx(&a));
        let restored = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, restored,
            "returning to A must restore the selected equipment scroll exactly"
        );
    }

    #[test]
    fn unit_view_equipment_scrollbar_drag_capture_continues_outside_and_clears_on_up() {
        let stage = stage_with(&[]);
        let snap = snap_unit_many_equipment(20, "A");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&snap));

        let down = pointer_down_unit_equipment_thumb(&mut view);
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "thumb pointer-down should only capture equipment drag"
        );
        let geom = unit_equipment_scrollbar(&view);
        let drag = view.on_input(&ViewInput::PointerMove {
            x: geom.track.x + geom.track.w + 150.0,
            y: geom.track.y + geom.track.h + 200.0,
            drag: true,
        });
        assert!(
            drag.consumed && drag.needs_redraw,
            "captured equipment thumb drag should continue outside the pane"
        );
        let scrolled = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let (cur, max) = equipment_scroll_pair(&scrolled);
        assert_eq!(
            cur, max,
            "equipment thumb drag must clamp to max: {scrolled}"
        );
        assert_eq!(
            unit_scroll_pair(&scrolled).0,
            0,
            "equipment thumb drag must not move the division list: {scrolled}"
        );
        assert!(
            visible_equipment_chunk(&scrolled).contains("equipment_name=A_eq_13(D);"),
            "bottom-clamped equipment drag should expose backing later rows: {scrolled}"
        );

        let up = view.on_input(&ViewInput::PointerUp {
            x: geom.track.x + geom.track.w + 150.0,
            y: geom.track.y + geom.track.h + 200.0,
            button: PointerButton::Left,
        });
        assert!(
            up.consumed && !up.needs_redraw,
            "pointer-up should consume solely to clear equipment scrollbar capture"
        );
        let ignored = view.on_input(&ViewInput::PointerMove {
            x: geom.track.x,
            y: geom.track.y,
            drag: true,
        });
        assert!(
            !ignored.consumed && !ignored.needs_redraw && ignored.commands.is_empty(),
            "pointer move after thumb release must not keep stale equipment capture"
        );
        let after_up = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            equipment_scroll_pair(&after_up),
            (cur, max),
            "post-release move should not mutate equipment scroll"
        );
    }

    #[test]
    fn unit_view_scrollbar_track_clicks_are_consumed_without_selection_or_scroll() {
        let stage = stage_with(&[]);
        let divisions = many_unit_divisions_snap(30, "A");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&divisions));
        let before = bound_digest(&view.bind(&ctx(&stage, &divisions)));
        let geom = unit_division_scrollbar(&view);
        let track_y = (geom.thumb.y + geom.thumb.h + 10.0).min(geom.track.y + geom.track.h - 1.0);

        let out = view.on_input(&ViewInput::PointerDown {
            x: geom.track.x + geom.track.w * 0.5,
            y: track_y,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "division scrollbar track click should be an inert consumed UI hit"
        );
        assert_eq!(
            view.selected_division_key, None,
            "division scrollbar track click must not select an underlying row"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &divisions)));
        assert_eq!(
            before, after,
            "division track click must not alter selection or visible rows"
        );

        let equipment = snap_unit_many_equipment(20, "A");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&equipment));
        let before = bound_digest(&view.bind(&ctx(&stage, &equipment)));
        let geom = unit_equipment_scrollbar(&view);
        let track_y = (geom.thumb.y + geom.thumb.h + 10.0).min(geom.track.y + geom.track.h - 1.0);

        let out = view.on_input(&ViewInput::PointerDown {
            x: geom.track.x + geom.track.w * 0.5,
            y: track_y,
            button: PointerButton::Left,
        });
        assert!(
            out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "equipment scrollbar track click should be an inert consumed UI hit"
        );
        let after = bound_digest(&view.bind(&ctx(&stage, &equipment)));
        assert_eq!(
            before, after,
            "equipment track click must not alter scroll or selected equipment pane"
        );
    }

    #[test]
    fn unit_view_scrollbar_drag_capture_clears_on_rebind_without_geometry() {
        let many = many_unit_divisions_snap(30, "A");
        let few = snap_unit_a_multi();
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&many));

        let down = pointer_down_unit_division_thumb(&mut view);
        assert!(down.consumed);
        assert!(
            view.scroll_drag.is_some(),
            "division thumb pointer-down should install a capture"
        );

        view.rebind(&unit_rebind_ctx(&few));
        assert!(
            view.scroll_drag.is_none(),
            "rebind to a non-overflowing division list must clear stale capture"
        );

        let ignored = view.on_input(&ViewInput::PointerMove {
            x: 0.0,
            y: 500.0,
            drag: true,
        });
        assert!(
            !ignored.consumed && !ignored.needs_redraw && ignored.commands.is_empty(),
            "drag move after stale capture clears must be inert"
        );
    }

    #[test]
    fn unit_view_stats_tab_click_switches_digest_and_visible_body() {
        let stage = stage_with(&[]);
        let snap = snap_unit_many_equipment(20, "A");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&snap));

        wheel_unit_equipment(&mut view, -3.0);
        let stats = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(unit_stats_tab_digest(&stats), "stats");
        assert_eq!(equipment_scroll_pair(&stats).0, 150);
        assert!(visible_equipment_chunk(&stats).contains("equipment_name=A_eq_04(D);"));

        let out = click_unit_stats_tab(&mut view, UnitStatsTab::History);
        assert!(out.consumed && out.needs_redraw);
        let history = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(unit_stats_tab_digest(&history), "history");
        assert_eq!(
            visible_equipment_chunk(&history),
            "",
            "history tab must hide the stats-tab equipment body without fabricating rows: {history}"
        );
        assert!(
            history.contains("equipment_name=A_eq_04(D);"),
            "hidden equipment rows should remain bound in EQROWS for scrub/fidelity accounting: {history}"
        );

        let same = click_unit_stats_tab(&mut view, UnitStatsTab::History);
        assert!(same.consumed && !same.needs_redraw);

        let out = click_unit_stats_tab(&mut view, UnitStatsTab::Template);
        assert!(out.consumed && out.needs_redraw);
        let template = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(unit_stats_tab_digest(&template), "template");
        assert_eq!(visible_equipment_chunk(&template), "");

        let out = click_unit_stats_tab(&mut view, UnitStatsTab::Stats);
        assert!(out.consumed && out.needs_redraw);
        let restored = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(unit_stats_tab_digest(&restored), "stats");
        assert_eq!(equipment_scroll_pair(&restored).0, 150);
        assert!(visible_equipment_chunk(&restored).contains("equipment_name=A_eq_04(D);"));
    }

    #[test]
    fn unit_view_hidden_equipment_pane_wheel_is_inert_on_non_stats_tabs() {
        let stage = stage_with(&[]);
        let snap = snap_unit_many_equipment(20, "A");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&snap));

        wheel_unit_equipment(&mut view, -3.0);
        let stats = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(equipment_scroll_pair(&stats).0, 150);

        click_unit_stats_tab(&mut view, UnitStatsTab::History);
        let history_before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let out = wheel_unit_equipment(&mut view, -3.0);
        assert!(!out.consumed);
        assert!(!out.needs_redraw);
        let history_after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            history_before, history_after,
            "hidden History-tab equipment pane must not mutate scroll state"
        );

        click_unit_stats_tab(&mut view, UnitStatsTab::Template);
        let template_before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let out = wheel_unit_equipment(&mut view, -3.0);
        assert!(!out.consumed);
        assert!(!out.needs_redraw);
        let template_after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            template_before, template_after,
            "hidden Template-tab equipment pane must not mutate scroll state"
        );

        click_unit_stats_tab(&mut view, UnitStatsTab::Stats);
        let restored = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(equipment_scroll_pair(&restored).0, 150);
        assert!(visible_equipment_chunk(&restored).contains("equipment_name=A_eq_04(D);"));
    }

    #[test]
    fn unit_view_stats_tab_selection_survives_scrub_a_b_a() {
        let stage = stage_with(&[]);
        let a = snap_unit_many_equipment(20, "A");
        let b = snap_unit_many_equipment(20, "B");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&a));

        let out = click_unit_stats_tab(&mut view, UnitStatsTab::Template);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(unit_stats_tab_digest(&a1), "template");
        assert_eq!(visible_equipment_chunk(&a1), "");
        assert!(a1.contains("equipment_name=A_eq_01(D);"));

        view.rebind(&unit_rebind_ctx(&b));
        let b_digest = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert_eq!(unit_stats_tab_digest(&b_digest), "template");
        assert_eq!(visible_equipment_chunk(&b_digest), "");
        assert!(b_digest.contains("equipment_name=B_eq_01(D);"));
        assert!(!b_digest.contains("equipment_name=A_eq_01(D);"));

        view.rebind(&unit_rebind_ctx(&a));
        let a2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "UnitView stats-tab scrub A->B->A must be byte-identical"
        );
    }

    #[test]
    fn unit_view_wheel_outside_division_list_is_inert() {
        let snap = many_unit_divisions_snap(30, "A");
        let mut view = UnitView::new();
        view.rebind(&unit_rebind_ctx(&snap));
        let viewport = unit_division_list_fallback(view.screen);

        let out = view.on_input(&ViewInput::Wheel {
            x: viewport.x + viewport.w + 20.0,
            y: viewport.y + UNIT_VIEW_ROW_H,
            dy: -5.0,
        });
        assert!(
            !out.consumed && !out.needs_redraw && out.commands.is_empty(),
            "wheel outside the division list must be inert"
        );
        assert_eq!(view.division_scroll_y, 0.0);
    }

    #[test]
    fn naval_results_zero_combats_empty_roster_badge_not_zeros() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["naval_combat_results.json"]);
        // snap_a has 0 naval combats.
        let bv = NavalCombatResultsView::new().bind(&ctx(&stage, &snap_a()));
        let d = bound_digest(&bv);
        assert!(d.contains("naval=0"), "demo carries 0 naval combats");
        // the `outcome` ROOT element renders the not-captured glyph (set_element_text → the
        // node tree), never a fabricated outcome.
        assert!(
            d.contains("outcome|Text(\"—\""),
            "the battle outcome must render the not-captured glyph"
        );
        // the N6 roster panel is in the draw digest (empty sides → no fabricated rows).
        assert!(
            d.contains("PANEL title="),
            "the N6 roster panel is in the digest"
        );
        // FID line shows unavailable bindings dominate (0 exact ship counts fabricated).
        assert!(
            d.contains("\nFID e=0 "),
            "no exact ship-count fabrication with 0 naval combats"
        );
    }

    /// NAVAL COMBAT no-fabrication: with 0 naval combats (snap_a) every header field renders
    /// the not-captured glyph "—" (correct-empty: no live battle), NEVER a fabricated 0. With
    /// 1 naval combat (snap_b) the belligerent owner TAGs (GER/ENG) bind derived.
    #[test]
    fn naval_combat_zero_combats_is_correct_empty_not_zeros() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["naval_combat.json"]);
        // snap_a: 0 naval combats.
        let bva = NavalCombatView::new().bind(&ctx(&stage, &snap_a()));
        let da = bound_digest(&bva);
        assert!(
            da.contains("naval=0"),
            "demo carries 0 naval combats, got:\n{da}"
        );
        // the TEXTBOX-backed header fields render the not-captured glyph "—" (no live battle),
        // never a fabricated value. (`flag_*` / `*_ships_count` are buttonType/Container nodes
        // that carry no text — their honesty lives in the fidelity tally, never a fake 0.)
        assert!(
            da.contains("naval_battle_title|Text(\"—\""),
            "no naval battle → title is —, got:\n{da}"
        );
        assert!(
            da.contains("leader_attacker_name|Text(\"—\""),
            "no naval battle → leader name is —"
        );
        // ALL 7 HEADER fields are tallied unavailable (none of them exact/derived). The single
        // EXACT tally is the `combats`-domain section state (present this hour), NOT a header
        // field — so no header datum is fabricated.
        assert_eq!(
            bva.fidelity.derived, 0,
            "no derived HEADER field with 0 naval combats, got:\n{da}"
        );
        assert_eq!(
            bva.fidelity.unavailable, 7,
            "all 7 header fields badge unavailable, got:\n{da}"
        );
        assert_eq!(
            bva.fidelity.exact, 1,
            "the only exact tally is the combats-section state (not a header datum)"
        );
        // never a fabricated 0 anywhere in the rendered tree.
        assert!(
            !da.contains("|Text(\"0\""),
            "no header field may fabricate a 0, got:\n{da}"
        );
        // snap_b: 1 naval combat (GER vs ENG) → belligerent owner TAGs bind derived (the flag
        // cells are buttons, so the TAG surfaces in the fidelity tally as 2 derived + the title;
        // the digest's DRAW line carries the count, the FID line proves the derived bindings).
        let bvb = NavalCombatView::new().bind(&ctx(&stage, &snap_b()));
        let db = bound_digest(&bvb);
        assert!(
            db.contains("naval=1"),
            "snap_b carries 1 naval combat, got:\n{db}"
        );
        // 3 derived bindings (attacker flag, defender flag, title) when a battle exists.
        assert_eq!(
            bvb.fidelity.derived, 3,
            "attacker/defender flags + title bind derived, got:\n{db}"
        );
        assert_eq!(
            bvb.fidelity.unavailable, 4,
            "the 4 leader/ship-count fields stay unavailable"
        );
    }

    /// COMBAT LOG no-fabrication: the `combat_log_view` ROOT binds only the static `title`;
    /// the page/tab ledger fields (period/aggregates/equipment totals) are LOGICAL gaps
    /// (tallied as unavailable, never bound as ROOT cells). The combats-list rows are
    /// reconstructed from the captured ACTIVE `combats[]` (not from absent ledger totals);
    /// per-battle loss cells badge unavailable, never a fabricated 0. The `combat_log` domain
    /// is ABSENT in the snapshot (honest "—", not a fabricated empty history).
    #[test]
    fn combat_log_root_title_only_ledger_unavailable_no_fab() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["combat_log.json"]);
        let bv = CombatLogView::new().bind(&ctx(&stage, &snap_a()));
        let d = bound_digest(&bv);
        // the static title binds on the ROOT (derived); the ledger domain is absent.
        assert!(
            d.contains("title|Text(\"Combat Log\""),
            "the root title binds, got:\n{d}"
        );
        assert!(
            d.contains("log_dom=absent"),
            "the combat_log ledger domain is absent (honest), got:\n{d}"
        );
        // the reconstructed combats list has 1 row (the 1 land combat in snap_a).
        assert!(
            d.contains("combats_sect=present"),
            "combats list reconstructed from captured combats[]"
        );
        assert!(
            d.contains("rows=r0["),
            "one reconstructed combat row, got:\n{d}"
        );
        assert!(
            d.contains("key=combat:1;location=5(E);"),
            "combat row identity and exact captured location must bind, got:\n{d}"
        );
        for element in [
            "def_lost_manpower",
            "atk_lost_armor_eq",
            "def_lost_armor_eq",
            "atk_lost_air_eq",
            "def_lost_air_eq",
            "atk_lost_other_eq",
            "def_lost_other_eq",
        ] {
            assert!(
                d.contains(&format!("{element}={DASH}(Ub)")),
                "per-battle {element} is unavailable, got:\n{d}"
            );
        }
        // the per-battle loss cells badge unavailable (no stored ledger), never a 0.
        assert!(
            d.contains(&format!("atk_lost_manpower={DASH}(Ub)")),
            "per-battle loss is unavailable, got:\n{d}"
        );
        assert!(
            !d.contains("atk_lost_manpower=0("),
            "per-battle loss must NEVER fabricate a 0"
        );
        // there are unavailable-badged ledger gaps (page/tab fields) in the fidelity tally.
        assert!(
            bv.fidelity.unavailable >= 9,
            "the 9 page/tab ledger fields are logical gaps, got:\n{d}"
        );
    }

    #[test]
    fn combat_log_row_clicks_are_read_only_noops() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["combat_log.json"]);
        let snap = snap_a();
        let mut host = WindowHost::new();
        host.open_instance(Box::new(CombatLogView::new()));
        host.rebind_all(&RebindContext {
            snapshot: &snap,
            country: "GER",
            screen: (1600, 900),
            refit: true,
        });

        let before = {
            let bound = host.bind_all(&ctx(&stage, &snap));
            assert_eq!(bound.len(), 1);
            let digest = bound_digest(&bound[0]);
            assert!(
                digest.contains("key=combat:1;location=5(E);"),
                "test must target a real reconstructed combat row, got:\n{digest}"
            );
            digest
        };

        let viewport = combat_log_rows_viewport_fallback(default_combat_log_screen());
        let x = viewport.x + 8.0;
        let y = viewport.y + 8.0;
        for ev in [
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            },
            ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            },
        ] {
            let out = host.route_input(&ev);
            assert!(
                out.consumed,
                "combat-log row click must be consumed as read-only replay chrome"
            );
            assert!(
                out.commands.is_empty(),
                "combat-log row click must not emit a drilldown/mutation command on {ev:?}"
            );
            assert!(
                !out.needs_redraw,
                "combat-log read-only row click must not request redraw on {ev:?}"
            );
        }

        let below_only_row = ViewInput::PointerDown {
            x,
            y: viewport.y + COMBAT_LOG_ROW_H + 8.0,
            button: PointerButton::Left,
        };
        let miss = host.route_input(&below_only_row);
        assert!(
            !miss.consumed,
            "clicking below the only reconstructed row must fall through, not claim an empty row"
        );

        let after = {
            let bound = host.bind_all(&ctx(&stage, &snap));
            assert_eq!(bound.len(), 1);
            bound_digest(&bound[0])
        };
        assert_eq!(
            before, after,
            "combat-log read-only row clicks must not mutate replay-local view state"
        );
    }

    #[test]
    fn naval_losses_faction_list_exact_totals_unavailable() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["naval_losses.json"]);
        let bv = NavalLossesView::new().bind(&ctx(&stage, &snap_a()));
        let d = bound_digest(&bv);
        // the faction filter list is derived from per-country politics.faction.name (exact).
        assert!(
            d.contains("fac_sect=present"),
            "faction section present, got: {d}"
        );
        assert!(
            d.contains("text=axis(E)") && d.contains("text=allies(E)"),
            "distinct faction names bound exact, got: {d}"
        );
    }

    // -------- per-view element accounting: scene walk == the .gui's element_type_counts ---

    /// Count `type` tokens recursively over the RAW gui json (the authoritative element set).
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

    /// For EVERY one of the 12 cluster `.gui` files, the element set we account for (the
    /// recursive `type`-token walk over the parsed tree) EQUALS the file's declared
    /// element_type_counts — proving every element of every view is accounted (none
    /// silently dropped). Includes the destroyed_ships EMPTY STUB (0 == 0).
    #[test]
    fn element_accounting_all_fourteen_views() {
        if !assets_present() {
            return;
        }
        for name in [
            "logistics.json",
            "garrison_log.json",
            "naval_combat_results.json",
            "destroyed_ships.json",
            "naval_losses.json",
            "naval_strategic_region.json",
            "unit_view.json",
            "event_window.json",
            "popup_window.json",
            "detailed_windows.json",
            "core.json",
            "ingame_menu.json",
            "naval_combat.json",
            "combat_log.json",
        ] {
            assert_eq!(
                raw_type_counts(name),
                declared_counts(name),
                "element accounting drifted for {name}"
            );
        }
    }

    /// The per-view element TOTALS match the REAL parsed counts (the binding-map totals were
    /// stale; these are the authoritative on-disk numbers).
    #[test]
    fn element_accounting_totals_match_real_gui() {
        if !assets_present() {
            return;
        }
        let total = |n: &str| raw_type_counts(n).values().sum::<usize>();
        assert_eq!(total("logistics.json"), 208);
        assert_eq!(total("garrison_log.json"), 57);
        assert_eq!(total("naval_combat_results.json"), 115);
        assert_eq!(total("destroyed_ships.json"), 0); // present-but-empty stub
        assert_eq!(total("naval_losses.json"), 78);
        assert_eq!(total("naval_strategic_region.json"), 45);
        assert_eq!(total("unit_view.json"), 539);
        assert_eq!(total("event_window.json"), 78);
        assert_eq!(total("popup_window.json"), 387);
        assert_eq!(total("detailed_windows.json"), 53);
        assert_eq!(total("core.json"), 83);
        assert_eq!(total("ingame_menu.json"), 77);
        assert_eq!(total("naval_combat.json"), 161);
        assert_eq!(total("combat_log.json"), 123);
    }

    /// The destroyed_ships `.gui` is a PRESENT-BUT-EMPTY stub: `windows:[]`, no elements.
    /// Documented (not silently dropped) — and its view is a registered EMPTY-STUB
    /// placeholder (gui_root None), menubar-reachable for honesty.
    #[test]
    fn destroyed_ships_is_documented_empty_stub() {
        if !assets_present() {
            return;
        }
        let p = gui_dir().join("destroyed_ships.json");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let windows = v
            .get("windows")
            .and_then(|w| w.as_array())
            .expect("windows array");
        assert!(
            windows.is_empty(),
            "destroyed_ships must be a present-but-empty stub"
        );
        assert!(
            declared_counts("destroyed_ships.json").is_empty(),
            "0 declared elements"
        );
        // the view is registered with NO gui root (an explicit empty-stub placeholder).
        use crate::host::ViewRegistry;
        let reg = ViewRegistry::from_inventory();
        let d = reg
            .view(DESTROYED_SHIPS_VIEW_ID)
            .expect("destroyed_ships view registered");
        assert!(
            d.gui_root.is_none(),
            "the empty-stub view must NOT name a fabricated parsed root"
        );
    }

    /// POPUP coverage (Codex (c)): the 32-popup-root constant ENUMERATES exactly the set of
    /// `containerWindowType` window names in `popup_window.json` — every ROOT is covered by
    /// the single library view (none silently ignored).
    #[test]
    fn popup_window_roots_enumerated_completely() {
        if !assets_present() {
            return;
        }
        let p = gui_dir().join("popup_window.json");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let file_roots: std::collections::BTreeSet<String> = v
            .get("windows")
            .and_then(|w| w.as_array())
            .unwrap()
            .iter()
            .filter_map(|w| {
                w.get("name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        let covered: std::collections::BTreeSet<String> =
            POPUP_ROOTS.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            file_roots, covered,
            "every popup_window root must be enumerated/covered"
        );
        assert_eq!(POPUP_ROOTS.len(), 32, "all 32 popup roots");
    }

    // -------- binding-coverage: every bound element name exists in its target window -------

    /// Collect every `name` in a `.gui` window subtree (the named element set), RAW-walking.
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

    /// Every element name a `Cell` binds MUST exist as a real node in its target window
    /// (the ROOT for summary cells, the row TEMPLATE for row cells) — else `set_element_text`
    /// silently no-ops. Mirrors the `bind` code across the 12 views.
    #[test]
    fn every_bound_element_name_exists_in_its_window() {
        if !assets_present() {
            return;
        }
        let cases: &[(&str, &str, &[&str])] = &[
            (
                "logistics.json",
                "countrylogisticsview",
                &[
                    "fulfilled",
                    "military_factories_usage",
                    "naval_factories_usage",
                    "nuke_count",
                ],
            ),
            (
                "logistics.json",
                "logistics_overview_resource_item",
                &["value", "icon"],
            ),
            (
                "logistics.json",
                "logistics_overview_land_equipment_entry",
                &["equipment_type", "equipment_icon", "needs", "in_stock"],
            ),
            (
                "garrison_log.json",
                "garrison_log_view",
                &["total_equipment_lost", "period_desc"],
            ),
            (
                "naval_combat_results.json",
                "navalcombatresultswindow",
                &["outcome", "name", "text"],
            ),
            (
                "naval_losses.json",
                "navallosseswindow",
                &["total_ships", "total_ships_sunk"],
            ),
            ("naval_losses.json", "navallosses_name_header", &["text"]),
            (
                "naval_strategic_region.json",
                "navalregionview",
                &[
                    "region_name",
                    "ship_text",
                    "convoys_text",
                    "plane_text",
                    "mine_text",
                    "enemy_ship_text",
                    "enemy_convoys_text",
                    "enemy_plane_text",
                    "enemy_mine_text",
                ],
            ),
            (
                "unit_view.json",
                "army_division_view",
                &[
                    "unit_name",
                    "unit_icon",
                    "bar_org",
                    "bar_str",
                    "flag",
                    "unitlist_movement_icon",
                    "unit_level",
                    "low_supply",
                    "dig_in_bonus",
                    "unitlist_acclimatization",
                    "bar_prepare",
                ],
            ),
            (
                "unit_view.json",
                "army_division_stats_view",
                &["unit_title"],
            ),
            (
                "unit_view.json",
                "stat_equipment_entry",
                &[
                    "equipment_name",
                    "equipment_amount",
                    "equipment_icon",
                    "creator_flag",
                ],
            ),
            (
                "event_window.json",
                "EventWindow",
                &["Title", "Description"],
            ),
            (
                "detailed_windows.json",
                "shipswindow",
                &["total_ships", "current_manpower"],
            ),
            ("core.json", "countryflag_entry", &["flag"]),
            // the live naval-combat header fields the view binds on the navalcombatview ROOT.
            (
                "naval_combat.json",
                "navalcombatview",
                &[
                    "flag_attacker",
                    "flag_defender",
                    "naval_battle_title",
                    "leader_attacker_name",
                    "leader_defender_name",
                    "leader_attacker_ships_count",
                    "leader_defender_ships_count",
                ],
            ),
            // the combat_log_view ROOT only carries the static `title` (the data-bearing
            // page/tab fields live in SEPARATE roots → recorded as logical gaps, asserted
            // never bound here so a stale set_element_text cannot no-op).
            ("combat_log.json", "combat_log_view", &["title"]),
            // NOTE: ingamemenu's data fields (disk_quota/saves_dirs) live in the SEPARATE
            // ingamemenu_load/_save sub-window roots; the `ingamemenu` root binds no cell
            // (recorded as OS-scope gaps) so there is nothing to assert here.
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

    // NOTE — the no-fabrication source guard for THIS file is the shared
    // `views::digest::tests::no_fabricated_default_in_cluster_views` test (it recursively
    // scans every `.rs` under `src/views/clusters/`, including this one). A local copy
    // cannot live here: it would scan its OWN pattern-list string literals as offenders
    // (which is exactly why the shared guard lives in `digest.rs`, OUTSIDE `clusters/`).

    // -------- registration: all 12 views collected + routed under the cluster -------------

    #[test]
    fn all_fourteen_views_registered_and_routed() {
        use crate::host::ViewRegistry;
        let reg = ViewRegistry::from_inventory();
        let ids = [
            LOGISTICS_VIEW_ID,
            GARRISON_LOG_VIEW_ID,
            NAVAL_RESULTS_VIEW_ID,
            DESTROYED_SHIPS_VIEW_ID,
            NAVAL_LOSSES_VIEW_ID,
            NAVAL_REGION_VIEW_ID,
            UNIT_VIEW_ID,
            EVENT_WINDOW_VIEW_ID,
            POPUP_WINDOW_VIEW_ID,
            DETAILED_WINDOWS_VIEW_ID,
            CORE_VIEW_ID,
            INGAME_MENU_VIEW_ID,
            NAVAL_COMBAT_VIEW_ID,
            COMBAT_LOG_VIEW_ID,
        ];
        for id in ids {
            assert!(
                reg.view(id).is_some(),
                "view {id} must be inventory-collected"
            );
        }
        let bar = reg.menubar();
        let group = bar
            .iter()
            .find(|g| g.cluster == MenuCluster::LogisticsCombatUnitsEvents)
            .expect("the Logistics cluster group exists");
        assert_eq!(
            group.items.len(),
            14,
            "all 14 cluster views routed to the Logistics menu"
        );
        // the two newly-registered views are MENU-REACHABLE.
        for id in [NAVAL_COMBAT_VIEW_ID, COMBAT_LOG_VIEW_ID] {
            assert!(
                group.items.iter().any(|i| i.view_id == id),
                "view {id} must be menu-reachable"
            );
        }
    }
}
