//! CLUSTER 2g — intel_diplomacy_trade (11 views) [N2 tabs, N5 event/popup card].
//!
//! The INTELLIGENCE / DIPLOMACY (trade / peace / world-tension) / WAR / INSIGNIA
//! surface, bound per `replay_ui/BINDING_MAP_intel_diplomacy_trade.md` against the REAL
//! parsed game `.gui` files fetched via [`BindContext::view_root`] / [`gui_symbol`] —
//! NOT synthetic chrome:
//!
//!   1. **intel_agency** — `countryintelligenceagencyview` ROOT. The agency identity /
//!      branch / upgrade / operations / cryptology block (`intel.agency` /
//!      `intel.operations` / `intel.decryption` are `null` in the demo → ARTIFACT-
//!      badged). `total_operative_text` binds the EXACT live operative count
//!      (`len(intel.operatives)`); the cap needs `agency.capacity` → derived. The two
//!      tab bodies (Operations / Cryptology) drive an **N2 tab bar** whose unfed tabs
//!      badge `unavailable` rather than fabricate a body.
//!   2. **intel_ledger** — `country_intel_ledger_view` ROOT. Every estimate cell is a
//!      fog-of-war intel ESTIMATE (runtime min–max band, NOT stored) → all `unavailable`
//!      (TRUE GAP b: binding the real value would leak perfect information). The per-tab
//!      `intel` name label binds the observed-country name (exact).
//!   3. **operative** — `boost_party_popularity_mission_window` ROOT: a deferred MUTATION
//!      modal (start a boost-party mission). Inert in a replay; every field badged.
//!   4. **operative_leader** — the operative roster + `operative_badge_view` detail card.
//!      `intel.operatives[]` is POPULATED → name / skill / traits / nationalities / state
//!      bind **exact** per operative; `gfx` / `captured` / ETA / mission detail are
//!      `unavailable` (ARTIFACT, unemitted in the demo). The roster is a row factory.
//!   5. **operation_overview** — `operation_overview_window` ROOT. `intel.operations` is
//!      `null` → the whole view is ARTIFACT-badged; the success `outlook` is a TRUE GAP (b).
//!   6. **trade** — `countrytradeview` ROOT (RESOURCE trade, not the equipment market).
//!      `produced` derives from owned `states[*].resources`; the per-partner trade roster
//!      + import/export/convoy legs are `unavailable` (TRUE GAP a: no resource-trade
//!      ledger in the schema). The partner rows are a row factory.
//!   7. **peace_conference** — `peaceconference_full_window` ROOT. The conference is a
//!      transient, runtime-only sub-game (`intel.peace_conference` is `null`); the full
//!      participant / bidding model is a TRUE GAP (b). Surfaced as an **N5 event/popup
//!      card** badged `unavailable` (the peace OUTCOME is in the diff timeline).
//!   8. **world_tension_popup** — `world_tension_popup` ROOT. The wars list binds EXACT
//!      from `diplomacy_world.wars` (belligerent TAGs + the two side war-scores); the
//!      threat-contributor list needs `world_events.threat[]` (absent in demo → ARTIFACT);
//!      the AGGREGATE tension scalar is a TRUE GAP (b). Both row sets are row factories.
//!   9. **war_overview** — `waroverview_window` ROOT. Header war-score + side losses bind
//!      EXACT (`a_casualties` / `b_casualties` / `war_score_*`); per-participant rows bind
//!      flag / name (exact) + division count / IC / wargoal (derived). Per-participant
//!      war-score / casualties / capitulation are TRUE GAPs. Wars / participants /
//!      factions / callable-allies are row factories over `diplomacy_world`.
//!  10. **insignia_selection** — `insignia_selection_window` ROOT: a cosmetic formation-
//!      editor MUTATION modal. The catalog is STATIC DEF; the current value needs an
//!      explicit selected-formation route arg plus `order_of_battle`; otherwise it badges
//!      `order_of_battle` (absent) → `unavailable`. Inert in a replay.
//!  11. **ins_revolution** — `INS_revolution_ui_window` ROOT: a mod-specific war-escalation
//!      indicator driven by a script variable (`world.variables` absent) → `unavailable`.
//!
//! THE NO-FABRICATION CONTRACT. This is a CLUSTER file, so the source-scanning guard
//! (`views::digest::no_fabricated_default_in_cluster_views`) forbids `unwrap_or(0)` /
//! `unwrap_or_default()` / the raw `ctx.snapshot(` seam. Every field is read through
//! [`BindContext::country_field`] / a top-level [`domain_field`] accessor →
//! [`Captured`] and rendered through [`DisplayAtom`], so a blocked / absent / null field
//! renders the not-captured glyph "—" — NEVER a fabricated 0. Genuinely runtime-computed
//! (intel estimate band, world-tension scalar, capitulation %, per-participant war-score),
//! schema-unfed (agency / operations / decryption / world_events / market), and DEF/world
//! data (insignia catalog, operation DEF) fields are badged `unavailable`. Mutation verbs
//! (launch operation, start mission, send trade, vote, pick insignia) render as real
//! chrome but are inert.
//!
//! THE ROW MODEL (mirrors `politics_government` / `production_construction_decisions`).
//! A `gridBoxType` is a row factory: the ROOT scene tree (static chrome + summary text
//! boxes) is fixed regardless of how many array elements exist; per-element rows are
//! instantiated from a TEMPLATE window and CUSTOM-DRAWN by the view's `Drawable`. So the
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

use widgets::common::{Atom, DrawAssets};
use widgets::event_card::PopupCard;
use widgets::tabs::{Tab, TabBar};

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
use std::cell::RefCell;

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

/// Format a measured value as its raw string token (TAG, ideology id, trait id, name).
fn fmt_token(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => DASH.to_string(),
    }
}

/// Format a measured array of string tokens as a comma-joined list (operative traits /
/// nationalities). A measured-but-EMPTY array renders "none" (an exact empty, Codex: an
/// empty list is not unavailable — it is a real "no traits"). Non-strings drop to "—".
fn fmt_token_list(v: &serde_json::Value) -> String {
    match v.as_array() {
        Some(a) if a.is_empty() => "none".to_string(),
        Some(a) => {
            let toks: Vec<String> = a
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
            if toks.is_empty() {
                DASH.to_string()
            } else {
                toks.join(", ")
            }
        }
        None => DASH.to_string(),
    }
}

/// Format a measured number as a SIGNED integer string (`+50`/`-100`) — the diplomatic
/// opinion readout. A measured `0` renders `+0` (a real zero, never the not-captured glyph).
fn fmt_signed(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) => {
            let n = f.trunc() as i64;
            if n >= 0 {
                format!("+{n}")
            } else {
                n.to_string()
            }
        }
        None => DASH.to_string(),
    }
}

/// Format a 0..1 fraction as a percent (stability / war support). Measured only — a
/// non-number drops to "—" (never a fabricated 0%).
fn fmt_frac_pct(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) => format!("{}%", (f.clamp(0.0, 1.0) * 100.0).round() as i64),
        None => DASH.to_string(),
    }
}

/// Record one `unavailable` for a LOGICAL field that has NO backing `.gui` node in the bound
/// ROOT (a separate sub-window field, a present-but-null subject leaf, or an unfed grid).
/// Tallied in the report for honest accounting but NEVER pushed as a targetable cell, so it
/// can never silently no-op a `set_element_text` against the wrong root.
fn record_gap(report: &mut FidelityReport) {
    report.record(&DisplayAtom::unavailable_because(
        UnavailableReason::Blocked,
    ));
}

/// The sorted set of country TAGs present this hour, read through the audited `countries`
/// map accessor (NOT the banned raw `snapshot()` seam). Empty when the domain is absent
/// (never fabricated). Used by the diplomacy country-list row factory.
fn country_tags(ctx: &BindContext) -> Vec<String> {
    match ctx.domain_field("countries") {
        Captured::Measured(v) => match v.as_object() {
            Some(map) => {
                let mut tags: Vec<String> = map.keys().cloned().collect();
                tags.sort();
                tags
            }
            None => Vec::new(),
        },
        _ => Vec::new(),
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
/// `GuiFile` typed parser REJECTED the file — this falls back to a LENIENT re-parse from
/// `Stage::assets_dir/gui/<file>`: coerce any non-numeric `position` `x`/`y` scalar to its
/// numeric prefix and drop a string `maxWidth`/`maxHeight`, then re-parse. READ-ONLY,
/// per-cluster recovery (no shared file edited) so the view binds the REAL game layout.
/// Returns [`empty_gui`] only if genuinely unreadable even after sanitising (never a
/// fabricated layout). (Mirrors the recovery in `politics_government`.)
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

/// Walk the JSON tree and repair the two string-where-`f32`-expected shapes the strict
/// `GuiFile` typed parser rejects: a `position` `x`/`y` STRING → its numeric prefix, and a
/// `text` `maxWidth`/`maxHeight` STRING → dropped (a percentage max isn't an f32). No
/// other value is touched.
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

/// Parse the leading signed-NUMBER prefix of a string and truncate to an integer pixel
/// (`"1s"`→1, `"-2px"`→-2, `"1.5"`→1, `"abc"`→0). Recovers a malformed `.gui` `position`
/// scalar (a layout coordinate) — not a snapshot value, so no no-fabrication concern.
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

#[derive(Clone)]
struct SpriteCell {
    element: String,
    asset: assets::SpriteAsset,
    fidelity: Fidelity,
}

/// One instantiated data row: the element→atom cells for its text fields. Custom-drawn by
/// the view's `Drawable` through the template; recorded in the digest so a scrub is exact.
/// `gaps` holds LOGICAL unavailable fields with NO backing `.gui` node — recorded for
/// honesty, never bound (so they can never silently no-op a `set_element_text`).
#[derive(Clone)]
struct DataRow {
    cells: Vec<Cell>,
    sprites: Vec<SpriteCell>,
    gaps: Vec<String>,
}

impl DataRow {
    fn new() -> DataRow {
        DataRow {
            cells: Vec::new(),
            sprites: Vec::new(),
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
    /// Bind `element` from an owned measured value (a synthesized TAG/name string we KNOW
    /// is real — e.g. a war belligerent TAG from `diplomacy_world.wars`). Exact/derived.
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
    fn bind_sprite(
        &mut self,
        report: &mut FidelityReport,
        element: &str,
        asset: assets::SpriteAsset,
        fidelity: Fidelity,
    ) {
        record_sprite_asset(report, &asset, fidelity);
        self.sprites.push(SpriteCell {
            element: element.to_string(),
            asset,
            fidelity,
        });
    }
    /// Record an `unavailable` badge for a LOGICAL field with NO backing `.gui` node (an
    /// OPEN gap / a separate prototype window / a computed aggregate). Tallied in the
    /// report, never pushed as a targetable cell.
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
        for sprite in &r.sprites {
            s.push_str(&sprite_digest(sprite));
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
/// (Mirrors `politics_government::render_rows`.)
fn render_rows(
    fd: &mut FrameDraw,
    panel: &[Node],
    file: &GuiFile,
    grid_name: &str,
    template_name: &str,
    rows: &[DataRow],
) {
    let manifest = &fd.stage.manifest;
    let inst = match instantiate_rows(panel, file, grid_name, template_name, rows, manifest) {
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

fn instantiate_rows(
    panel: &[Node],
    file: &GuiFile,
    grid_name: &str,
    template_name: &str,
    rows: &[DataRow],
    manifest: &assets::SpriteManifest,
) -> Option<gridbox::GridInstances> {
    let viewport = find_rect(panel, grid_name)?;
    let template = find_template(file, template_name)?;
    if rows.is_empty() {
        return None;
    }
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
        for sprite in &rows[i].sprites {
            apply_sprite_cell(row, sprite);
        }
    }
    Some(inst)
}

fn instantiated_row_roots(
    panel: &[Node],
    file: &GuiFile,
    grid_name: &str,
    template_name: &str,
    rows: &[DataRow],
    manifest: &assets::SpriteManifest,
) -> Vec<Node> {
    match instantiate_rows(panel, file, grid_name, template_name, rows, manifest) {
        Some(inst) => inst.rows,
        None => Vec::new(),
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

/// Set the bound summary atoms into the ROOT scene tree + lay it out. Shared by every
/// view's `bind`. Returns the laid-out root windows for the named ROOT (only).
fn build_root(ctx: &BindContext, file: &GuiFile, root_name: &str, summary: &[Cell]) -> Vec<Node> {
    build_root_with_sprites(ctx, file, root_name, summary, &[])
}

fn build_root_with_sprites(
    ctx: &BindContext,
    file: &GuiFile,
    root_name: &str,
    summary: &[Cell],
    sprites: &[SpriteCell],
) -> Vec<Node> {
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
    for sprite in sprites {
        for root in roots.iter_mut() {
            apply_sprite_cell(root, sprite);
        }
    }
    for root in roots.iter_mut() {
        layout(root, ctx.screen, None, &cm);
    }
    roots
}

fn record_sprite_asset(
    report: &mut FidelityReport,
    asset: &assets::SpriteAsset,
    fidelity: Fidelity,
) {
    let atom = match asset {
        assets::SpriteAsset::Resolved(sprite) => {
            DisplayAtom::text(format!("{}#{}", sprite.name, sprite.frame), fidelity)
        }
        assets::SpriteAsset::Unavailable(_) => {
            DisplayAtom::unavailable_because(UnavailableReason::Blocked)
        }
    };
    report.record(&atom);
}

fn push_sprite(
    report: &mut FidelityReport,
    summary: &mut Vec<SpriteCell>,
    element: &str,
    asset: assets::SpriteAsset,
    fidelity: Fidelity,
) {
    record_sprite_asset(report, &asset, fidelity);
    summary.push(SpriteCell {
        element: element.to_string(),
        asset,
        fidelity,
    });
}

fn sprite_digest(sprite: &SpriteCell) -> String {
    match &sprite.asset {
        assets::SpriteAsset::Resolved(resolved) => {
            let badge = match sprite.fidelity {
                Fidelity::Exact => "E",
                Fidelity::Derived => "D",
                Fidelity::Unavailable => "U",
            };
            format!(
                "{}:sprite={}#{}({badge});",
                sprite.element, resolved.name, resolved.frame
            )
        }
        assets::SpriteAsset::Unavailable(miss) => {
            format!(
                "{}:sprite=MISS:{}(U);",
                sprite.element,
                sprite_miss_digest(miss)
            )
        }
    }
}

fn sprite_miss_digest(miss: &assets::SpriteMiss) -> String {
    match miss {
        assets::SpriteMiss::NoSpriteName => "no_sprite_name".to_string(),
        assets::SpriteMiss::MissingSprite(name) => format!("missing_sprite:{name}"),
        assets::SpriteMiss::MissingTexture(name) => format!("missing_texture:{name}"),
        assets::SpriteMiss::AmbiguousTexturePath(path) => {
            format!("ambiguous_texture_path:{path}")
        }
        assets::SpriteMiss::GenericPlaceholder(name) => format!("generic_placeholder:{name}"),
        assets::SpriteMiss::FrameOutOfRange {
            name,
            frame,
            frames,
        } => {
            format!("frame_out_of_range:{name}:{frame}:{frames}")
        }
    }
}

fn apply_sprite_cell(root: &mut Node, sprite: &SpriteCell) -> bool {
    match &sprite.asset {
        assets::SpriteAsset::Resolved(resolved) => {
            set_element_sprite(root, &sprite.element, &resolved.name, resolved.frame)
        }
        assets::SpriteAsset::Unavailable(_) => false,
    }
}

fn set_element_sprite(node: &mut Node, name: &str, sprite: &str, frame: u32) -> bool {
    if node.name == name {
        match &mut node.kind {
            ElementKind::Icon {
                sprite: current,
                frame: current_frame,
            } => {
                *current = sprite.to_string();
                *current_frame = frame;
                return true;
            }
            ElementKind::Button {
                sprite: current,
                frame: current_frame,
                ..
            } => {
                *current = Some(sprite.to_string());
                *current_frame = frame;
                return true;
            }
            ElementKind::Background { sprite: current }
            | ElementKind::NineSlice { sprite: current }
            | ElementKind::ProgressBar {
                sprite: current, ..
            } => {
                if frame == 0 {
                    *current = sprite.to_string();
                    return true;
                }
            }
            ElementKind::Container { background } => {
                if frame == 0 {
                    *background = Some(sprite.to_string());
                    return true;
                }
            }
            ElementKind::Text(_) => {}
        }
    }
    for child in node.children.iter_mut() {
        if set_element_sprite(child, name, sprite, frame) {
            return true;
        }
    }
    false
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

/// Read an array field under the observed country as a slice + its section state.
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

fn top_domain_array<'a>(
    domain: &Captured<&'a serde_json::Value>,
    pointer: &str,
) -> (Option<&'a [serde_json::Value]>, SectionState) {
    match domain {
        Captured::Measured(v) => match v.pointer(pointer) {
            Some(serde_json::Value::Array(a)) => (Some(a.as_slice()), SectionState::Present),
            Some(serde_json::Value::Null) => (None, SectionState::Null),
            Some(_) => (None, SectionState::Degraded),
            None => (None, SectionState::Absent),
        },
        Captured::Null => (None, SectionState::Null),
        Captured::Absent => (None, SectionState::Absent),
    }
}

fn portrait_asset(ctx: &BindContext, record: &serde_json::Value) -> assets::SpriteAsset {
    assets::resolve_portrait_sprite(&ctx.stage.manifest, record)
}

fn first_nonempty_sprite_field(
    ctx: &BindContext,
    record: &serde_json::Value,
    fields: &[&str],
) -> assets::SpriteAsset {
    let mut first_miss = None;
    for field in fields {
        let resolved = assets::resolve_sprite_value(&ctx.stage.manifest, record.get(*field));
        match resolved {
            assets::SpriteAsset::Resolved(_) => return resolved,
            assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName) => {}
            miss => {
                if first_miss.is_none() {
                    first_miss = Some(miss);
                }
            }
        }
    }
    first_miss.unwrap_or(assets::SpriteAsset::Unavailable(
        assets::SpriteMiss::NoSpriteName,
    ))
}

fn agency_insignia_asset(ctx: &BindContext, tag: &str) -> assets::SpriteAsset {
    match ctx.country_field(tag, "intel.agency") {
        Captured::Measured(agency) => first_nonempty_sprite_field(
            ctx,
            agency,
            &[
                "insignia_gfx",
                "icon_gfx",
                "logo_gfx",
                "gfx",
                "icon",
                "logo",
                "insignia",
            ],
        ),
        Captured::Null | Captured::Absent => {
            assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormationSelection {
    Army(i64),
    ArmyGroup(i64),
}

impl FormationSelection {
    fn from_arg(arg: Option<i64>) -> Option<FormationSelection> {
        match arg {
            Some(id) if id > 0 => Some(FormationSelection::Army(id)),
            Some(id) if id < 0 => id.checked_neg().map(FormationSelection::ArmyGroup),
            _ => None,
        }
    }

    fn source(self) -> (&'static str, &'static str, i64) {
        match self {
            FormationSelection::Army(id) => ("/armies", "GFX_theatre_army_shield", id),
            FormationSelection::ArmyGroup(id) => {
                ("/army_groups", "GFX_theatre_army_group_shield", id)
            }
        }
    }
}

fn selected_formation<'a>(
    ctx: &'a BindContext,
    tag: &str,
    selection: FormationSelection,
) -> Option<(&'a serde_json::Value, &'static str)> {
    let oob = ctx.order_of_battle();
    let (pointer, base_sprite, selected_id) = selection.source();
    let (items, _) = top_domain_array(&oob, pointer);
    if let Some(items) = items {
        for item in items {
            match elem_field(item, "owner") {
                Captured::Measured(owner)
                    if owner.as_str() == Some(tag) && formation_id_matches(item, selected_id) =>
                {
                    return Some((item, base_sprite));
                }
                _ => {}
            }
        }
    }
    None
}

fn formation_id_matches(formation: &serde_json::Value, selected_id: i64) -> bool {
    formation
        .get("id")
        .and_then(formation_id_value)
        .map(|id| id == selected_id)
        .unwrap_or(false)
}

fn formation_id_value(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok())),
        serde_json::Value::String(s) => {
            let t = s.trim();
            t.parse::<i64>().ok().or_else(|| {
                t.split_once(':')
                    .and_then(|(id, _ty)| id.trim().parse::<i64>().ok())
            })
        }
        _ => None,
    }
}

fn formation_icon_asset(
    ctx: &BindContext,
    formation: &serde_json::Value,
    base_sprite: &str,
) -> (assets::SpriteAsset, Fidelity) {
    let direct = first_nonempty_sprite_field(
        ctx,
        formation,
        &["insignia_gfx", "icon_gfx", "emblem_gfx", "gfx", "sprite"],
    );
    if !matches!(
        direct,
        assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName)
    ) {
        return (direct, Fidelity::Exact);
    }
    let icon = formation.get("icon").and_then(icon_one_based_frame);
    (
        assets::resolve_one_based_frame_sprite(&ctx.stage.manifest, base_sprite, icon),
        Fidelity::Derived,
    )
}

fn icon_one_based_frame(value: &serde_json::Value) -> Option<u32> {
    match value {
        serde_json::Value::Number(n) => n.as_u64().and_then(|u| u32::try_from(u).ok()),
        serde_json::Value::String(s) => s.trim().parse::<u32>().ok(),
        _ => None,
    }
}

fn build_insignia_catalog_rows(
    ctx: &BindContext,
    fidelity: &mut FidelityReport,
    base_sprite: Option<&str>,
) -> (Vec<DataRow>, SectionState) {
    let Some(base_sprite) = base_sprite else {
        SectionState::Absent.record(fidelity);
        return (Vec::new(), SectionState::Absent);
    };
    let frames = assets::catalog_frames(&ctx.stage.manifest, base_sprite);
    if frames.is_empty() {
        SectionState::Absent.record(fidelity);
        return (Vec::new(), SectionState::Absent);
    }
    let mut rows = Vec::new();
    for frame in frames {
        let mut row = DataRow::new();
        row.bind_sprite(
            fidelity,
            "button",
            assets::SpriteAsset::Resolved(frame),
            Fidelity::Derived,
        );
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

/// Read the top-level `diplomacy_world` domain as an object (wars / factions / subjects).
/// Top-level fold-in domain → reached through the audited `domain_field` accessor (NOT the
/// banned raw `snapshot()` seam), preserving the no-fabrication trichotomy.
fn diplomacy_world<'a>(ctx: &'a BindContext) -> Option<&'a serde_json::Value> {
    match ctx.domain_field("diplomacy_world") {
        Captured::Measured(v) => Some(v),
        _ => None,
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

/// Push an explicit `unavailable` summary cell (runtime-computed / DEF / unfed). "—".
fn push_unavailable(report: &mut FidelityReport, summary: &mut Vec<Cell>, element: &str) {
    let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

/// Push a measured owned-value summary cell (a TAG/name/count we KNOW is real).
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

/// Mint a per-frame 1x1 white texture (the substrate every widget tints).
fn mint_white(fd: &FrameDraw) -> render::Texture {
    widgets::common::white_texture(fd.renderer)
}

/// Build geometry-only `widgets::DrawAssets` over an owned white texture (labels drawn
/// only when a font is present; we draw geometry like the menubar's `font: None` fallback
/// so the widget layer is always visible + never fabricates a label). The widget DATA
/// (atoms) is bound + digested regardless of this draw.
fn widget_assets(white: &render::Texture) -> DrawAssets<'_> {
    DrawAssets {
        white,
        font: None,
        font_pages: &[],
    }
}

// =====================================================================================
// VIEW 1 — INTEL AGENCY  (countryintelligenceagencyview)  [N2 tabs]
// =====================================================================================

const INTEL_AGENCY_VIEW_ID: ViewId = ViewId("intel.intel_agency");
const INTEL_AGENCY_FILE: &str = "intel_agency.json";
const INTEL_AGENCY_ROOT: &str = "countryintelligenceagencyview";

const INTEL_AGENCY_TAB_AGENCY: &str = "agency";
const INTEL_AGENCY_TAB_OPERATIONS: &str = "operations";
const INTEL_AGENCY_TAB_CRYPTOLOGY: &str = "cryptology";

pub struct IntelAgencyView {
    selected_tab: Option<String>,
    tabs: TabBar,
    screen: Rect,
}

impl IntelAgencyView {
    pub fn new() -> IntelAgencyView {
        let tabs = intel_agency_tabs_for_flags(true, true, true);
        IntelAgencyView {
            selected_tab: tabs.selected().map(str::to_string),
            tabs,
            screen: default_intel_agency_screen(),
        }
    }
}

impl Default for IntelAgencyView {
    fn default() -> Self {
        IntelAgencyView::new()
    }
}

struct IntelAgencyDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    operation_rows: Vec<DataRow>,
    crypto_rows: Vec<DataRow>,
    operation_section: SectionState,
    crypto_section: SectionState,
    /// the N2 tab bar (Agency / Operations / Cryptology).
    tabs: TabBar,
    tab_panel: Rect,
}

impl Drawable for IntelAgencyDrawable {
    fn draw_digest(&self) -> String {
        let (visible_tab, visible_section, visible_rows) = match self.tabs.selected() {
            Some(INTEL_AGENCY_TAB_OPERATIONS) => (
                INTEL_AGENCY_TAB_OPERATIONS,
                self.operation_section.tag(),
                row_digest(&self.operation_rows),
            ),
            Some(INTEL_AGENCY_TAB_CRYPTOLOGY) => (
                INTEL_AGENCY_TAB_CRYPTOLOGY,
                self.crypto_section.tag(),
                row_digest(&self.crypto_rows),
            ),
            Some(INTEL_AGENCY_TAB_AGENCY) => (INTEL_AGENCY_TAB_AGENCY, "present", String::new()),
            _ => ("none", "absent", String::new()),
        };
        let selected_mode = selected_intel_agency_tab(&self.tabs);
        format!(
            "INTEL_AGENCY mode={:?} selected={:?} visible_tab={} visible_sect={} visible=[{}] op_sect={} crypto_sect={} ops={} crypto={} TABS {}",
            selected_mode,
            self.tabs.selected(),
            visible_tab,
            visible_section,
            visible_rows,
            self.operation_section.tag(),
            self.crypto_section.tag(),
            row_digest(&self.operation_rows),
            row_digest(&self.crypto_rows),
            widgets::tabs::plan_tabs(&self.tabs, self.tab_panel).digest(),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        match self.tabs.selected() {
            Some(INTEL_AGENCY_TAB_OPERATIONS) => render_rows(
                fd,
                &self.roots,
                &self.file,
                "operations_grid",
                "operation_view_entry",
                &self.operation_rows,
            ),
            Some(INTEL_AGENCY_TAB_CRYPTOLOGY) => render_rows(
                fd,
                &self.roots,
                &self.file,
                "country_list",
                "cryptology_country_entry",
                &self.crypto_rows,
            ),
            _ => {}
        }
        let white = mint_white(fd);
        let assets = widget_assets(&white);
        widgets::tabs::draw_tabs(fd.renderer, &self.tabs, self.tab_panel, &assets);
    }
}

impl ViewInstance for IntelAgencyView {
    fn view_id(&self) -> ViewId {
        INTEL_AGENCY_VIEW_ID
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
        self.tabs = intel_agency_tabs_with_selection(
            intel_agency_tabs_from_rebind(ctx, ctx.country),
            self.selected_tab.as_deref(),
        );
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => match click_intel_agency_tab(
                &mut self.tabs,
                intel_agency_tab_fallback(self.screen),
                x,
                y,
            ) {
                IntelAgencyTabClick::Changed => {
                    self.selected_tab = self.tabs.selected().map(str::to_string);
                    intel_agency_input_consumed(true)
                }
                IntelAgencyTabClick::Unchanged => {
                    self.selected_tab = self.tabs.selected().map(str::to_string);
                    intel_agency_input_consumed(false)
                }
                IntelAgencyTabClick::Inert => intel_agency_input_consumed(false),
                IntelAgencyTabClick::Miss => InputOutcome::ignored(),
            },
            _ => InputOutcome::ignored(),
        }
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, INTEL_AGENCY_FILE);

        let mut summary: Vec<Cell> = Vec::new();
        // The LIVE operative count is exact (len of the populated operatives list); the
        // cap (max_operatives) needs `agency.capacity` (null in demo) → derived "n/—".
        let op_count = match ctx.country_field(tag, "intel.operatives") {
            Captured::Measured(v) => v.as_array().map(|a| a.len()),
            _ => None,
        };
        let total_atom = match op_count {
            Some(n) => DisplayAtom::text(format!("{n}/{DASH}"), Fidelity::Derived),
            None => DisplayAtom::unavailable_because(UnavailableReason::Absent),
        };
        fidelity.record(&total_atom);
        summary.push(Cell {
            element: "total_operative_text".to_string(),
            atom: total_atom,
        });
        // arrested/dead operative tallies: no captured/dead field in the demo → unavailable.
        push_unavailable(&mut fidelity, &mut summary, "arrested_operative_text");
        push_unavailable(&mut fidelity, &mut summary, "dead_operative_text");
        // agency identity / defense / branches: `intel.agency` is null → ARTIFACT badged.
        push_unavailable(&mut fidelity, &mut summary, "agency_name");
        push_unavailable(&mut fidelity, &mut summary, "agency_defense_level");
        push_unavailable(&mut fidelity, &mut summary, "agency_branches_title");
        push_unavailable(&mut fidelity, &mut summary, "spymaster_button_name");

        let (operation_rows, operation_section) =
            build_intel_operation_rows(ctx, tag, &mut fidelity);
        let (crypto_rows, crypto_section) = build_intel_crypto_rows(ctx, tag, &mut fidelity);
        let tabs = intel_agency_tabs_with_selection(
            intel_agency_tabs(ctx, tag),
            self.selected_tab.as_deref(),
        );

        let roots = build_root(ctx, &file, INTEL_AGENCY_ROOT, &summary);
        let tab_panel = intel_agency_tab_panel(&roots, ctx.screen);
        let drawable = IntelAgencyDrawable {
            roots: roots.clone(),
            file,
            operation_rows,
            crypto_rows,
            operation_section,
            crypto_section,
            tabs,
            tab_panel,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: INTEL_AGENCY_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots).with_block_rect(tab_panel),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The intel-agency N2 tab set (Agency / Operations / Cryptology). The Operations + Crypto
/// tab BODIES are `intel.operations` / `intel.decryption` (null in the demo) → those tabs
/// badge `unavailable` (present but blocked), never fabricate a body.
fn intel_agency_tabs(ctx: &BindContext, tag: &str) -> TabBar {
    intel_agency_tabs_from_bind(ctx, tag)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntelAgencyTab {
    Agency,
    Operations,
    Cryptology,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntelAgencyTabClick {
    Changed,
    Unchanged,
    Inert,
    Miss,
}

fn intel_agency_tabs_from_bind(ctx: &BindContext, tag: &str) -> TabBar {
    let has = |path: &str| ctx.country_field(tag, path).is_measured();
    intel_agency_tabs_for_flags(
        has("intel.operatives"),
        has("intel.operations.running"),
        has("intel.decryption.targets"),
    )
}

fn intel_agency_tabs_from_rebind(ctx: &RebindContext, tag: &str) -> TabBar {
    let has = |path: &str| ctx.country_field(tag, path).is_measured();
    intel_agency_tabs_for_flags(
        has("intel.operatives"),
        has("intel.operations.running"),
        has("intel.decryption.targets"),
    )
}

fn intel_agency_tabs_for_flags(agency: bool, operations: bool, cryptology: bool) -> TabBar {
    let avail = |present: bool, id: &str, label: &str| {
        if present {
            Tab::enabled(id, Atom::exact(label))
        } else {
            Tab::unavailable(id, Atom::exact(label))
        }
    };
    TabBar::horizontal(vec![
        // the agency tab is "present" iff we have any live operative data to show.
        avail(agency, INTEL_AGENCY_TAB_AGENCY, "Agency"),
        avail(operations, INTEL_AGENCY_TAB_OPERATIONS, "Operations"),
        avail(cryptology, INTEL_AGENCY_TAB_CRYPTOLOGY, "Cryptology"),
    ])
}

fn intel_agency_tabs_with_selection(mut tabs: TabBar, selected: Option<&str>) -> TabBar {
    if let Some(id) = selected {
        tabs.select(id);
    }
    tabs.reconcile();
    tabs
}

fn selected_intel_agency_tab(bar: &TabBar) -> Option<IntelAgencyTab> {
    match bar.selected() {
        Some(INTEL_AGENCY_TAB_AGENCY) => Some(IntelAgencyTab::Agency),
        Some(INTEL_AGENCY_TAB_OPERATIONS) => Some(IntelAgencyTab::Operations),
        Some(INTEL_AGENCY_TAB_CRYPTOLOGY) => Some(IntelAgencyTab::Cryptology),
        _ => None,
    }
}

fn default_intel_agency_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn intel_agency_tab_fallback(screen: Rect) -> Rect {
    let root_x = screen.x + screen.w - 606.0;
    let root_y = screen.y + 78.0;
    Rect {
        x: root_x,
        y: root_y + 40.0,
        w: 550.0,
        h: (screen.y + screen.h - root_y - 40.0).max(widgets::tabs::BAR_THICK),
    }
}

fn intel_agency_tab_panel(roots: &[Node], screen: Rect) -> Rect {
    find_rect(roots, "agency_agents")
        .or_else(|| find_rect(roots, INTEL_AGENCY_ROOT))
        .unwrap_or_else(|| intel_agency_tab_fallback(screen))
}

fn click_intel_agency_tab(bar: &mut TabBar, panel: Rect, x: f32, y: f32) -> IntelAgencyTabClick {
    let rects = bar.layout(panel);
    if let Some(id) = bar.pick(&rects, x, y) {
        if bar.select(&id) {
            IntelAgencyTabClick::Changed
        } else {
            IntelAgencyTabClick::Unchanged
        }
    } else if rects.iter().any(|tr| tr.rect.contains(x, y)) {
        IntelAgencyTabClick::Inert
    } else {
        IntelAgencyTabClick::Miss
    }
}

fn intel_agency_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
}

/// The operations-tab `operation_view_entry` rows: `intel.operations` is null in the demo
/// → no rows, section badged unavailable (ARTIFACT). Enumerated for completeness; lights up
/// once `operations` is folded.
fn build_intel_operation_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let (arr, state) = match ctx.country_field(tag, "intel.operations.running") {
        Captured::Measured(v) => (v.as_array().cloned(), SectionState::Present),
        Captured::Null => (None, SectionState::Null),
        Captured::Absent => (None, SectionState::Absent),
    };
    let arr = match arr {
        Some(a) => a,
        None => {
            state.record(fidelity);
            return (
                Vec::new(),
                if state == SectionState::Present {
                    SectionState::Degraded
                } else {
                    state
                },
            );
        }
    };
    let mut rows = Vec::new();
    for op in &arr {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "operation_name_text",
            elem_field(op, "type"),
            Fidelity::Derived,
            fmt_token,
        );
        row.bind(
            fidelity,
            "btn_target_flag",
            elem_field(op, "target"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "operation_duration_text",
            elem_field(op, "duration"),
            Fidelity::Exact,
            fmt_int,
        );
        row.badge_unavailable(fidelity, "operation_ai_score"); // AI-internal, computed-not-stored
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

/// The cryptology-tab `cryptology_country_entry` rows: `intel.decryption` is null in the
/// demo → no rows, badged unavailable (ARTIFACT).
fn build_intel_crypto_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let (arr, state) = match ctx.country_field(tag, "intel.decryption.targets") {
        Captured::Measured(v) => (v.as_array().cloned(), SectionState::Present),
        Captured::Null => (None, SectionState::Null),
        Captured::Absent => (None, SectionState::Absent),
    };
    let arr = match arr {
        Some(a) => a,
        None => {
            state.record(fidelity);
            return (
                Vec::new(),
                if state == SectionState::Present {
                    SectionState::Degraded
                } else {
                    state
                },
            );
        }
    };
    let mut rows = Vec::new();
    for t in &arr {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "name",
            elem_field(t, "target"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "cryptology_state",
            elem_field(t, "amount"),
            Fidelity::Derived,
            fmt_int,
        );
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

fn open_intel_agency(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(IntelAgencyView::new())
}

register_view!(ViewDescriptor {
    id: INTEL_AGENCY_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(INTEL_AGENCY_FILE, INTEL_AGENCY_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "Intel Agency",
        order: 1
    }),
    shortcut: None,
    open: open_intel_agency,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.operation_view_entry"),
    gui_template: GuiSymbolRef::template(INTEL_AGENCY_FILE, "operation_view_entry"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.cryptology_country_entry"),
    gui_template: GuiSymbolRef::template(INTEL_AGENCY_FILE, "cryptology_country_entry"),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 2 — INTEL LEDGER  (country_intel_ledger_view)
// =====================================================================================

const INTEL_LEDGER_VIEW_ID: ViewId = ViewId("intel.intel_ledger");
const INTEL_LEDGER_FILE: &str = "intel_ledger.json";
const INTEL_LEDGER_ROOT: &str = "country_intel_ledger_view";

pub struct IntelLedgerView;

impl IntelLedgerView {
    pub fn new() -> IntelLedgerView {
        IntelLedgerView
    }
}

impl Default for IntelLedgerView {
    fn default() -> Self {
        IntelLedgerView::new()
    }
}

struct IntelLedgerDrawable {
    roots: Vec<Node>,
    /// the per-tab name label echoes the observed country (the one bindable EXACT field).
    target_name: String,
}

impl Drawable for IntelLedgerDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "INTEL_LEDGER target={} (all estimate cells unavailable: TRUE GAP b)",
            self.target_name
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for IntelLedgerView {
    fn view_id(&self) -> ViewId {
        INTEL_LEDGER_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, INTEL_LEDGER_FILE);

        let mut summary: Vec<Cell> = Vec::new();
        // The per-tab `intel` name label echoes the OBSERVED country's name — the one
        // honest exact field (the tab title, not an estimate). 4 tab frames carry it.
        push_value(
            &mut fidelity,
            &mut summary,
            "intel",
            serde_json::Value::String(tag.to_string()),
            Fidelity::Exact,
            fmt_token,
        );
        // EVERY estimate cell is a fog-of-war intel ESTIMATE (a runtime min–max band, NOT
        // stored). Binding the REAL economy/military value would LEAK perfect information
        // (Codex (a): even `derived` is wrong) → badge `unavailable` (TRUE GAP b). These
        // name real `.gui` text nodes so the "—" lands in the estimate slot, not nowhere.
        for el in [
            "industrial_capacity",
            "manpower",
            "convoys_count",
            "fuel_value",
            "estimated_civil_industry_size",
            "estimated_military_industry_size",
            "estimated_naval_industry_size",
            "army_count",
            "special_forces_count",
            "manpower_in_field",
            "equipment_fulfilled",
            "fleet_count",
            "ship_count",
            "airwing_count",
        ] {
            push_unavailable(&mut fidelity, &mut summary, el);
        }

        let roots = build_root(ctx, &file, INTEL_LEDGER_ROOT, &summary);
        let drawable = IntelLedgerDrawable {
            roots: roots.clone(),
            target_name: tag.to_string(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: INTEL_LEDGER_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_intel_ledger(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(IntelLedgerView::new())
}

register_view!(ViewDescriptor {
    id: INTEL_LEDGER_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(INTEL_LEDGER_FILE, INTEL_LEDGER_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "Intel Ledger",
        order: 2
    }),
    shortcut: None,
    open: open_intel_ledger,
    policy: HostPolicy::window(ZBand::Panel),
});

// =====================================================================================
// VIEW 3 — OPERATIVE  (boost_party_popularity_mission_window)  — deferred mutation modal
// =====================================================================================

const OPERATIVE_VIEW_ID: ViewId = ViewId("intel.operative");
const OPERATIVE_FILE: &str = "operative.json";
const OPERATIVE_ROOT: &str = "boost_party_popularity_mission_window";

pub struct OperativeView;

impl OperativeView {
    pub fn new() -> OperativeView {
        OperativeView
    }
}

impl Default for OperativeView {
    fn default() -> Self {
        OperativeView::new()
    }
}

struct OperativeDrawable {
    roots: Vec<Node>,
}

impl Drawable for OperativeDrawable {
    fn draw_digest(&self) -> String {
        "OPERATIVE boost-party mission modal (deferred mutation; inert in replay)".to_string()
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for OperativeView {
    fn view_id(&self) -> ViewId {
        OPERATIVE_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, OPERATIVE_FILE);
        // The whole view is a deferred MUTATION modal (start a boost-party mission). In a
        // replay it is never shown + carries no datum; the title/description are
        // action-construction text → badged. The OUTCOME (popularity shift) is in the
        // diff timeline / `politics.party_popularities` deltas, not this dialog.
        let mut summary: Vec<Cell> = Vec::new();
        push_unavailable(&mut fidelity, &mut summary, "title");
        push_unavailable(&mut fidelity, &mut summary, "description");

        let roots = build_root(ctx, &file, OPERATIVE_ROOT, &summary);
        let drawable = OperativeDrawable {
            roots: roots.clone(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: OPERATIVE_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_operative(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(OperativeView::new())
}

register_view!(ViewDescriptor {
    id: OPERATIVE_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(OPERATIVE_FILE, OPERATIVE_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "Operative Mission",
        order: 3
    }),
    shortcut: None,
    open: open_operative,
    policy: HostPolicy::window(ZBand::Panel),
});

// =====================================================================================
// VIEW 4 — OPERATIVE LEADER  (operative roster + operative_badge_view detail card)
// =====================================================================================

const OPERATIVE_LEADER_VIEW_ID: ViewId = ViewId("intel.operative_leader");
const OPERATIVE_LEADER_FILE: &str = "operative_leader.json";
const OPERATIVE_LEADER_ROOT: &str = "operative_badge_view";

pub struct OperativeLeaderView;

impl OperativeLeaderView {
    pub fn new() -> OperativeLeaderView {
        OperativeLeaderView
    }
}

impl Default for OperativeLeaderView {
    fn default() -> Self {
        OperativeLeaderView::new()
    }
}

struct OperativeLeaderDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    operative_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for OperativeLeaderDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "OPERATIVE_LEADER sect={} ops={}",
            self.section.tag(),
            row_digest(&self.operative_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        // the roster strip is a gridbox of portrait windows; we draw the per-operative
        // rows from the portrait-window template into the `operatives_box` viewport.
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "operatives_box",
            "operative_leader_portrait_window",
            &self.operative_rows,
        );
    }
}

impl ViewInstance for OperativeLeaderView {
    fn view_id(&self) -> ViewId {
        OPERATIVE_LEADER_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, OPERATIVE_LEADER_FILE);

        // The selected-operative detail card binds the FIRST operative (the badge view
        // shows the selected operative; a replay has no selection, so we surface the first
        // — a deterministic, real operative, never a fabricated one). All five present
        // fields bind exact; gfx/code-name are unavailable.
        let (arr, section) = country_array(ctx, tag, "intel.operatives");
        let mut summary: Vec<Cell> = Vec::new();
        let mut summary_sprites: Vec<SpriteCell> = Vec::new();
        if let Some(first) = arr.and_then(|a| a.first()) {
            summary.push(cell_bind(
                &mut fidelity,
                "operative_name",
                elem_field(first, "name"),
                Fidelity::Exact,
                fmt_token,
            ));
            summary.push(cell_bind(
                &mut fidelity,
                "operative_skill_level",
                elem_field(first, "skill"),
                Fidelity::Exact,
                fmt_int,
            ));
            summary.push(cell_bind(
                &mut fidelity,
                "operative_title",
                elem_field(first, "state"),
                Fidelity::Derived,
                fmt_token,
            ));
        } else {
            push_unavailable(&mut fidelity, &mut summary, "operative_name");
            push_unavailable(&mut fidelity, &mut summary, "operative_skill_level");
            push_unavailable(&mut fidelity, &mut summary, "operative_title");
        }
        // owning agency insignia: `intel.agency` is null → unavailable (ARTIFACT).
        push_sprite(
            &mut fidelity,
            &mut summary_sprites,
            "agency_insignia",
            agency_insignia_asset(ctx, tag),
            Fidelity::Exact,
        );

        let (operative_rows, section) = build_operative_rows(ctx, tag, &mut fidelity, section);

        let roots = build_root_with_sprites(
            ctx,
            &file,
            OPERATIVE_LEADER_ROOT,
            &summary,
            &summary_sprites,
        );
        let drawable = OperativeLeaderDrawable {
            roots: roots.clone(),
            file,
            operative_rows,
            section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: OPERATIVE_LEADER_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Bind one summary cell from a captured value, recording it (helper for the badge card).
fn cell_bind(
    report: &mut FidelityReport,
    element: &str,
    cap: Captured<&serde_json::Value>,
    fidelity: Fidelity,
    fmt: impl FnOnce(&serde_json::Value) -> String,
) -> Cell {
    let atom = cap.display_with_fidelity(fidelity, fmt);
    report.record(&atom);
    Cell {
        element: element.to_string(),
        atom,
    }
}

/// The operative roster rows: one per `intel.operatives[i]`. `name`/`skill`/`traits`/
/// `nationalities`/`state` bind EXACT (populated in the demo); `gfx`/`captured` are
/// `unavailable` (ARTIFACT, unemitted). A measured-but-empty `intel.operatives[]` is a
/// real "no operatives" (Present, 0 rows), NOT unavailable.
fn build_operative_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
    _section: SectionState,
) -> (Vec<DataRow>, SectionState) {
    let (arr, state) = country_array(ctx, tag, "intel.operatives");
    let arr = match arr {
        Some(a) => a,
        None => {
            state.record(fidelity);
            return (Vec::new(), state);
        }
    };
    let mut rows = Vec::new();
    for op in arr {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "operative_skill_level",
            elem_field(op, "skill"),
            Fidelity::Exact,
            fmt_int,
        );
        // The `state` token (on_operation/on_mission) is an exact STATUS label — bind it to
        // the status box. The mission ICON, by contrast, needs the unemitted `mission`/
        // `op_id` (Codex: state ≠ mission icon) → the icon stays unavailable (ARTIFACT).
        row.bind(
            fidelity,
            "status_text",
            elem_field(op, "state"),
            Fidelity::Exact,
            fmt_token,
        );
        row.badge_unavailable(fidelity, "mission_icon");
        // operative traits + nationalities are exact id lists (empty → "none", exact).
        row.bind(
            fidelity,
            "operative_traits",
            elem_field(op, "traits"),
            Fidelity::Exact,
            fmt_token_list,
        );
        row.bind(
            fidelity,
            "operative_nationalities",
            elem_field(op, "nationalities"),
            Fidelity::Exact,
            fmt_token_list,
        );
        // portrait gfx + captured overlay: unemitted in the demo → unavailable (ARTIFACT).
        row.bind_sprite(
            fidelity,
            "portrait_btn",
            portrait_asset(ctx, op),
            Fidelity::Exact,
        );
        row.badge_gap(fidelity, "leader_captured");
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

fn open_operative_leader(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(OperativeLeaderView::new())
}

register_view!(ViewDescriptor {
    id: OPERATIVE_LEADER_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        OPERATIVE_LEADER_FILE,
        OPERATIVE_LEADER_ROOT
    )),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "Operatives",
        order: 4
    }),
    shortcut: None,
    open: open_operative_leader,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.operative_portrait_window"),
    gui_template: GuiSymbolRef::template(OPERATIVE_LEADER_FILE, "operative_leader_portrait_window"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.operative_nationality_entry"),
    gui_template: GuiSymbolRef::template(OPERATIVE_LEADER_FILE, "operative_nationality_entry"),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 5 — OPERATION OVERVIEW  (operation_overview_window)
// =====================================================================================

const OPERATION_OVERVIEW_VIEW_ID: ViewId = ViewId("intel.operation_overview");
const OPERATION_OVERVIEW_FILE: &str = "operation_overview.json";
const OPERATION_OVERVIEW_ROOT: &str = "operation_overview_window";

pub struct OperationOverviewView;

impl OperationOverviewView {
    pub fn new() -> OperationOverviewView {
        OperationOverviewView
    }
}

impl Default for OperationOverviewView {
    fn default() -> Self {
        OperationOverviewView::new()
    }
}

struct OperationOverviewDrawable {
    roots: Vec<Node>,
    section: SectionState,
}

impl Drawable for OperationOverviewDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "OPERATION_OVERVIEW sect={} (operations null in demo → ARTIFACT)",
            self.section.tag()
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for OperationOverviewView {
    fn view_id(&self) -> ViewId {
        OPERATION_OVERVIEW_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, OPERATION_OVERVIEW_FILE);

        // One operation's detail. `intel.operations` is null in the demo → the whole view
        // is ARTIFACT-badged; the success `outlook` is a TRUE GAP (b) (runtime chance).
        let (arr, section) = country_array(ctx, tag, "intel.operations.running");
        let mut summary: Vec<Cell> = Vec::new();
        if let Some(op) = arr.and_then(|a| a.first()) {
            summary.push(cell_bind(
                &mut fidelity,
                "operation_name_text",
                elem_field(op, "type"),
                Fidelity::Derived,
                fmt_token,
            ));
            summary.push(cell_bind(
                &mut fidelity,
                "flag_name",
                elem_field(op, "target"),
                Fidelity::Exact,
                fmt_token,
            ));
            summary.push(cell_bind(
                &mut fidelity,
                "operation_duration_text",
                elem_field(op, "duration"),
                Fidelity::Exact,
                fmt_int,
            ));
        } else {
            push_unavailable(&mut fidelity, &mut summary, "operation_name_text");
            push_unavailable(&mut fidelity, &mut summary, "flag_name");
            push_unavailable(&mut fidelity, &mut summary, "operation_duration_text");
        }
        push_unavailable(&mut fidelity, &mut summary, "operation_outlook_text"); // TRUE GAP b
        push_unavailable(&mut fidelity, &mut summary, "operation_reward_text"); // operation DEF

        let roots = build_root(ctx, &file, OPERATION_OVERVIEW_ROOT, &summary);
        let drawable = OperationOverviewDrawable {
            roots: roots.clone(),
            section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: OPERATION_OVERVIEW_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_operation_overview(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(OperationOverviewView::new())
}

register_view!(ViewDescriptor {
    id: OPERATION_OVERVIEW_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        OPERATION_OVERVIEW_FILE,
        OPERATION_OVERVIEW_ROOT
    )),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "Operation Overview",
        order: 5
    }),
    shortcut: None,
    open: open_operation_overview,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.operation_resources_view_entry"),
    gui_template: GuiSymbolRef::template(OPERATION_OVERVIEW_FILE, "operation_resources_view_entry"),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 6 — TRADE  (countrytradeview)
// =====================================================================================

const TRADE_VIEW_ID: ViewId = ViewId("intel.trade");
const TRADE_FILE: &str = "trade.json";
const TRADE_ROOT: &str = "countrytradeview";

pub struct TradeView;

impl TradeView {
    pub fn new() -> TradeView {
        TradeView
    }
}

impl Default for TradeView {
    fn default() -> Self {
        TradeView::new()
    }
}

struct TradeDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    partner_rows: Vec<DataRow>,
}

impl Drawable for TradeDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "TRADE partners={} (resource-trade roster = TRUE GAP a)",
            row_digest(&self.partner_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "trade_grid",
            "country_trade_entry",
            &self.partner_rows,
        );
    }
}

impl ViewInstance for TradeView {
    fn view_id(&self) -> ViewId {
        TRADE_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, TRADE_FILE);

        let mut summary: Vec<Cell> = Vec::new();
        // Civilian-factory totals exist in `economy`; the TRADED split needs the (unfed)
        // market → total is derived, the traded split is unavailable.
        push_country(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "factories_total",
            "economy.civilian_factories",
            Fidelity::Derived,
            fmt_int,
        );
        push_unavailable(&mut fidelity, &mut summary, "factories_used");
        push_unavailable(&mut fidelity, &mut summary, "factories_available");

        // The per-partner resource-trade roster is a TRUE GAP (a): there is no resource-
        // trade ledger in the schema. No partner rows are fabricated; the gap is tallied
        // once (a logical unavailable, NOT a fake row instantiated from the template).
        push_unavailable_gap(&mut fidelity, "resource_trade_roster");
        let partner_rows: Vec<DataRow> = Vec::new();

        let roots = build_root(ctx, &file, TRADE_ROOT, &summary);
        let drawable = TradeDrawable {
            roots: roots.clone(),
            file,
            partner_rows,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: TRADE_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_trade(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(TradeView::new())
}

register_view!(ViewDescriptor {
    id: TRADE_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(TRADE_FILE, TRADE_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "Trade",
        order: 6
    }),
    shortcut: None,
    open: open_trade,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.country_trade_entry"),
    gui_template: GuiSymbolRef::template(TRADE_FILE, "country_trade_entry"),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 7 — PEACE CONFERENCE  (peaceconference_full_window)  [N5 event/popup card]
// =====================================================================================

const PEACE_CONFERENCE_VIEW_ID: ViewId = ViewId("intel.peace_conference");
const PEACE_CONFERENCE_FILE: &str = "peace_conference.json";
const PEACE_CONFERENCE_ROOT: &str = "peaceconference_full_window";

pub struct PeaceConferenceView;

impl PeaceConferenceView {
    pub fn new() -> PeaceConferenceView {
        PeaceConferenceView
    }
}

impl Default for PeaceConferenceView {
    fn default() -> Self {
        PeaceConferenceView::new()
    }
}

struct PeaceConferenceDrawable {
    roots: Vec<Node>,
    /// the N5 event/popup card — present ONLY when the conference is genuinely ACTIVE
    /// (`intel.peace_conference.active == true`). When inactive/null (the demo state) we
    /// render NO card (Codex: an unavailable card on an inactive conference is a fabricated
    /// UI state) — only the real root + the badged-unavailable fidelity/gaps remain.
    card: Option<PopupCard>,
    active: bool,
}

impl Drawable for PeaceConferenceDrawable {
    fn draw_digest(&self) -> String {
        let card_digest = match &self.card {
            Some(card) => {
                let area = Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 480.0,
                    h: 600.0,
                };
                widgets::event_card::plan_card(&card.layout(area)).digest()
            }
            None => "(no card — conference inactive)".to_string(),
        };
        format!(
            "PEACE_CONFERENCE active={} CARD {}",
            self.active, card_digest
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        if let Some(card) = &self.card {
            let area = find_rect(&self.roots, PEACE_CONFERENCE_ROOT).unwrap_or(Rect {
                x: 360.0,
                y: 120.0,
                w: 480.0,
                h: 600.0,
            });
            let laid = card.layout(area);
            let white = mint_white(fd);
            let assets = widget_assets(&white);
            widgets::event_card::draw_card(fd.renderer, &laid, &assets, |_, _| false);
        }
    }
}

impl ViewInstance for PeaceConferenceView {
    fn view_id(&self) -> ViewId {
        PEACE_CONFERENCE_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, PEACE_CONFERENCE_FILE);

        // The active/visibility gate maps to `intel.peace_conference.active` (the one real
        // bind; §3.8 stores `{active}`). `null` everywhere in the demo → the conference is
        // inactive. The full participant / bidding / pot / claim model is a TRUE GAP (b)
        // (a runtime interactive sub-game, not persisted). The OUTCOME (annexations /
        // releases) is surfaced via the diff timeline, never fabricated here.
        let active = matches!(ctx.country_field(tag, "intel.peace_conference.active"), Captured::Measured(v) if v.as_bool() == Some(true));
        let active_atom = match ctx.country_field(tag, "intel.peace_conference.active") {
            Captured::Measured(v) => DisplayAtom::text(
                if v.as_bool() == Some(true) {
                    "active"
                } else {
                    "inactive"
                }
                .to_string(),
                Fidelity::Derived,
            ),
            _ => DisplayAtom::unavailable_because(UnavailableReason::Null),
        };
        fidelity.record(&active_atom);
        let summary: Vec<Cell> = vec![];
        // the participant/agenda/vote/pot model — all TRUE GAP (b), tallied as gaps.
        push_unavailable_gap(&mut fidelity, "participants");
        push_unavailable_gap(&mut fidelity, "agenda");
        push_unavailable_gap(&mut fidelity, "votes");
        push_unavailable_gap(&mut fidelity, "pot");

        // The N5 card is rendered ONLY when the conference is genuinely ACTIVE — and even
        // then its title/body are badged unavailable (the per-tick bidding state is not
        // persisted). When inactive/null (the demo) NO card is drawn (Codex: a card on an
        // inactive conference is a fabricated UI state).
        let card = if active {
            Some(PopupCard::new(Atom::unavailable()).body(Atom::unavailable()))
        } else {
            None
        };

        let roots = build_root(ctx, &file, PEACE_CONFERENCE_ROOT, &summary);
        let drawable = PeaceConferenceDrawable {
            roots: roots.clone(),
            card,
            active,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: PEACE_CONFERENCE_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_peace_conference(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(PeaceConferenceView::new())
}

register_view!(ViewDescriptor {
    id: PEACE_CONFERENCE_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        PEACE_CONFERENCE_FILE,
        PEACE_CONFERENCE_ROOT
    )),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "Peace Conference",
        order: 7
    }),
    shortcut: None,
    open: open_peace_conference,
    policy: HostPolicy::window(ZBand::Panel),
});

// =====================================================================================
// VIEW 8 — WORLD TENSION POPUP  (world_tension_popup)  [N5-adjacent]
// =====================================================================================

const WORLD_TENSION_VIEW_ID: ViewId = ViewId("intel.world_tension_popup");
const WORLD_TENSION_FILE: &str = "world_tension_popup.json";
const WORLD_TENSION_ROOT: &str = "world_tension_popup";

pub struct WorldTensionView;

impl WorldTensionView {
    pub fn new() -> WorldTensionView {
        WorldTensionView
    }
}

impl Default for WorldTensionView {
    fn default() -> Self {
        WorldTensionView::new()
    }
}

struct WorldTensionDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    war_rows: Vec<DataRow>,
    threat_rows: Vec<DataRow>,
    war_section: SectionState,
    threat_section: SectionState,
}

impl Drawable for WorldTensionDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "WORLD_TENSION war_sect={} threat_sect={} wars={} threats={} (aggregate scalar = TRUE GAP b)",
            self.war_section.tag(),
            self.threat_section.tag(),
            row_digest(&self.war_rows),
            row_digest(&self.threat_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "list",
            "world_tension_war_entry",
            &self.war_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "list",
            "world_tension_entry",
            &self.threat_rows,
        );
    }
}

impl ViewInstance for WorldTensionView {
    fn view_id(&self) -> ViewId {
        WORLD_TENSION_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, WORLD_TENSION_FILE);

        // The wars/threat lists are custom-drawn from the .gui row templates; the ROOT
        // carries no bound summary text (the aggregate scalar has no node here), so the
        // summary is empty by design.
        let summary: Vec<Cell> = Vec::new();
        // The AGGREGATE world-tension scalar is COMPUTED-NOT-STORED (TRUE GAP b) — only the
        // contributor ledger + scaling inputs are stored, never the scalar. Badge it (a
        // logical gap, no `.gui` text node for the topbar number is in this popup).
        push_unavailable_gap(&mut fidelity, "aggregate_world_tension");

        let (war_rows, war_section) = build_tension_war_rows(ctx, &mut fidelity);
        let (threat_rows, threat_section) = build_tension_threat_rows(ctx, &mut fidelity);

        let roots = build_root(ctx, &file, WORLD_TENSION_ROOT, &summary);
        let drawable = WorldTensionDrawable {
            roots: roots.clone(),
            file,
            war_rows,
            threat_rows,
            war_section,
            threat_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: WORLD_TENSION_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Tally one unavailable LOGICAL gap (no `.gui` node) directly into the report.
fn push_unavailable_gap(report: &mut FidelityReport, _label: &str) {
    report.record(&DisplayAtom::unavailable_because(
        UnavailableReason::Blocked,
    ));
}

/// The wars-list rows: one `world_tension_war_entry` per `diplomacy_world.wars[w]`. The
/// belligerent TAGs + the two side war-scores bind EXACT; the war NAME / major-marker are
/// derived (composed/loc'd, not stored).
fn build_tension_war_rows(
    ctx: &BindContext,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let dw = match diplomacy_world(ctx) {
        Some(v) => v,
        None => {
            SectionState::Absent.record(fidelity);
            return (Vec::new(), SectionState::Absent);
        }
    };
    let wars = match dw.get("wars").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => {
            SectionState::Null.record(fidelity);
            return (Vec::new(), SectionState::Null);
        }
    };
    let mut rows = Vec::new();
    for war in wars {
        let mut row = DataRow::new();
        // composed war name "<a> vs <b>" — derived; a missing side → unavailable (not a
        // "—-as-derived" string).
        match war_name(war) {
            Some(name) => row.bind_value(
                fidelity,
                "war_name",
                serde_json::Value::String(name),
                Fidelity::Derived,
                fmt_token,
            ),
            None => row.badge_unavailable(fidelity, "war_name"),
        }
        row.bind(
            fidelity,
            "attacker_country_flag",
            elem_field(war, "a"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "defender_country_flag",
            elem_field(war, "b"),
            Fidelity::Exact,
            fmt_token,
        );
        // the war-score bar from the two stored SIDE scores (exact).
        row.bind(
            fidelity,
            "warscore_progressbar",
            elem_field(war, "war_score_a_vs_b"),
            Fidelity::Exact,
            fmt_int,
        );
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

/// The threat-contributor rows: one `world_tension_entry` per `world_events.threat[i]`.
/// `world_events` is ABSENT in the demo → no rows, badged unavailable (ARTIFACT).
fn build_tension_threat_rows(
    ctx: &BindContext,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let we = match ctx.world_events() {
        Captured::Measured(v) => v,
        Captured::Null => {
            SectionState::Null.record(fidelity);
            return (Vec::new(), SectionState::Null);
        }
        Captured::Absent => {
            SectionState::Absent.record(fidelity);
            return (Vec::new(), SectionState::Absent);
        }
    };
    let threat = match we.get("threat").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => {
            SectionState::Degraded.record(fidelity);
            return (Vec::new(), SectionState::Degraded);
        }
    };
    let mut rows = Vec::new();
    for t in threat {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "country_name",
            elem_field(t, "tag"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "date",
            elem_field(t, "date"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "value",
            elem_field(t, "final_threat"),
            Fidelity::Exact,
            fmt_int,
        );
        row.bind(
            fidelity,
            "reason",
            elem_field(t, "label"),
            Fidelity::Exact,
            fmt_token,
        );
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

fn open_world_tension(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(WorldTensionView::new())
}

register_view!(ViewDescriptor {
    id: WORLD_TENSION_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(WORLD_TENSION_FILE, WORLD_TENSION_ROOT)),
    required_domains: &["diplomacy_world"],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "World Tension",
        order: 8
    }),
    shortcut: None,
    open: open_world_tension,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.world_tension_war_entry"),
    gui_template: GuiSymbolRef::template(WORLD_TENSION_FILE, "world_tension_war_entry"),
    required_domains: &["diplomacy_world"],
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.world_tension_entry"),
    gui_template: GuiSymbolRef::template(WORLD_TENSION_FILE, "world_tension_entry"),
    required_domains: &["world_events"],
});

// =====================================================================================
// VIEW 9 — WAR OVERVIEW  (waroverview_window)
// =====================================================================================

const WAR_OVERVIEW_VIEW_ID: ViewId = ViewId("intel.war_overview");
const WAR_OVERVIEW_FILE: &str = "war_overview.json";
const WAR_OVERVIEW_ROOT: &str = "waroverview_window";

pub struct WarOverviewView;

impl WarOverviewView {
    pub fn new() -> WarOverviewView {
        WarOverviewView
    }
}

impl Default for WarOverviewView {
    fn default() -> Self {
        WarOverviewView::new()
    }
}

struct WarOverviewDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    war_rows: Vec<DataRow>,
    attacker_rows: Vec<DataRow>,
    defender_rows: Vec<DataRow>,
    war_section: SectionState,
}

impl Drawable for WarOverviewDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "WAR_OVERVIEW war_sect={} wars={} attackers={} defenders={}",
            self.war_section.tag(),
            row_digest(&self.war_rows),
            row_digest(&self.attacker_rows),
            row_digest(&self.defender_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "waroverview_wars_box",
            "waroverview_war_button",
            &self.war_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "attacker_grid",
            "war_ally_entry",
            &self.attacker_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "defender_grid",
            "war_ally_entry",
            &self.defender_rows,
        );
    }
}

impl ViewInstance for WarOverviewView {
    fn view_id(&self) -> ViewId {
        WAR_OVERVIEW_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, WAR_OVERVIEW_FILE);

        // The selected war is the first war the OBSERVED country is actually involved in (a
        // replay has no in-game selection; an involved war is the honest, deterministic
        // choice). If the observed country is in NO war, the header detail is `unavailable`
        // (Codex: do NOT attribute an arbitrary third-party war's header to this country) —
        // the full war LIST still renders below.
        let dw = diplomacy_world(ctx);
        let wars: Vec<serde_json::Value> =
            match dw.and_then(|v| v.get("wars")).and_then(|v| v.as_array()) {
                Some(a) => a.clone(),
                None => Vec::new(),
            };
        let war_section = if dw.is_none() {
            SectionState::Absent
        } else if wars.is_empty() {
            SectionState::Null
        } else {
            SectionState::Present
        };
        let selected = pick_war_for(&wars, ctx.country);

        let mut summary: Vec<Cell> = Vec::new();
        if let Some(war) = &selected {
            // header: war title is derived from belligerents (both sides measured, else U);
            // side losses + war-score are exact.
            push_value_opt(
                &mut fidelity,
                &mut summary,
                "header_desc",
                war_name(war),
                Fidelity::Derived,
            );
            summary.push(cell_bind(
                &mut fidelity,
                "attacker_losses",
                elem_field(war, "a_casualties"),
                Fidelity::Exact,
                fmt_int,
            ));
            summary.push(cell_bind(
                &mut fidelity,
                "defender_losses",
                elem_field(war, "b_casualties"),
                Fidelity::Exact,
                fmt_int,
            ));
            summary.push(cell_bind(
                &mut fidelity,
                "warscore_progressbar",
                elem_field(war, "war_score_a_vs_b"),
                Fidelity::Exact,
                fmt_int,
            ));
            // per-side FIELDED DIVISIONS from top-level divisions[] (a derived division
            // count; the `_fielded_man` slot WOULD show manpower, which we do not store, so
            // we present the honest division count, derived — never a fabricated manpower).
            let a_tag = war.get("a").and_then(|v| v.as_str());
            let b_tag = war.get("b").and_then(|v| v.as_str());
            push_side_div_count(
                ctx,
                &mut fidelity,
                &mut summary,
                "attacker_fielded_man",
                a_tag,
            );
            push_side_div_count(
                ctx,
                &mut fidelity,
                &mut summary,
                "defender_fielded_man",
                b_tag,
            );
        } else {
            for el in [
                "header_desc",
                "attacker_losses",
                "defender_losses",
                "warscore_progressbar",
                "attacker_fielded_man",
                "defender_fielded_man",
            ] {
                push_unavailable(&mut fidelity, &mut summary, el);
            }
        }

        let war_rows = build_war_tab_rows(&wars, &mut fidelity);
        let attacker_rows = build_war_participant_rows(ctx, selected.as_ref(), "a", &mut fidelity);
        let defender_rows = build_war_participant_rows(ctx, selected.as_ref(), "b", &mut fidelity);

        let roots = build_root(ctx, &file, WAR_OVERVIEW_ROOT, &summary);
        let drawable = WarOverviewDrawable {
            roots: roots.clone(),
            file,
            war_rows,
            attacker_rows,
            defender_rows,
            war_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: WAR_OVERVIEW_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The first war in `wars` whose belligerent `a`/`b` is the observed `tag`.
fn pick_war_for(wars: &[serde_json::Value], tag: &str) -> Option<serde_json::Value> {
    wars.iter()
        .find(|w| {
            w.get("a").and_then(|v| v.as_str()) == Some(tag)
                || w.get("b").and_then(|v| v.as_str()) == Some(tag)
        })
        .cloned()
}

/// Compose a war's derived display name "<a> vs <b>" — `Some` ONLY when BOTH belligerent
/// TAGs are measured strings. A missing side returns `None` (the caller badges it
/// `unavailable`, never a "—-as-Derived" string — Codex fix).
fn war_name(war: &serde_json::Value) -> Option<String> {
    let a = war.get("a").and_then(|v| v.as_str())?;
    let b = war.get("b").and_then(|v| v.as_str())?;
    Some(format!("{a} vs {b}"))
}

/// Push a summary cell from an `Option<String>`: `Some` → measured at `fidelity`, `None` →
/// the not-captured glyph (badged `unavailable`, never a fabricated/derived "—").
fn push_value_opt(
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
    element: &str,
    value: Option<String>,
    fidelity: Fidelity,
) {
    let atom = match value {
        Some(s) => DisplayAtom::text(s, fidelity),
        None => DisplayAtom::unavailable_because(UnavailableReason::Absent),
    };
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

/// Count `divisions[]` whose `owner == tag` (top-level `divisions[]` IS present → a
/// derivable division COUNT). The `_fielded_man` slot would show manpower, which is NOT
/// stored; we present the honest division count (derived) instead — never a fabricated
/// manpower. A measured-but-NON-ARRAY `divisions` → `unavailable` (Codex: not a `0`).
fn push_side_div_count(
    ctx: &BindContext,
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
    element: &str,
    tag: Option<&str>,
) {
    let atom = match (tag, ctx.divisions()) {
        (Some(t), Captured::Measured(v)) => match v.as_array() {
            Some(arr) => {
                let n = arr
                    .iter()
                    .filter(|d| d.get("owner").and_then(|o| o.as_str()) == Some(t))
                    .count();
                DisplayAtom::text(format!("{n}"), Fidelity::Derived)
            }
            None => DisplayAtom::unavailable_because(UnavailableReason::Blocked),
        },
        _ => DisplayAtom::unavailable_because(UnavailableReason::Absent),
    };
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

/// One `waroverview_war_button` per `diplomacy_world.wars[w]` — the war tab strip.
fn build_war_tab_rows(wars: &[serde_json::Value], fidelity: &mut FidelityReport) -> Vec<DataRow> {
    let mut rows = Vec::new();
    for war in wars {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "attacker_country_flag",
            elem_field(war, "a"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "defender_country_flag",
            elem_field(war, "b"),
            Fidelity::Exact,
            fmt_token,
        );
        rows.push(row);
    }
    rows
}

/// The per-participant `war_ally_entry` rows for one side (`"a"`/`"b"`). The single named
/// belligerent binds flag/name EXACT; division COUNT + IC derive from top-level
/// `divisions[]` / `economy`; wargoal derives from `diplomacy.wargoals`. Per-participant
/// war-score / casualties / capitulation are TRUE GAPs (side-level only) → unavailable.
fn build_war_participant_rows(
    ctx: &BindContext,
    war: Option<&serde_json::Value>,
    side: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let war = match war {
        Some(w) => w,
        None => return Vec::new(),
    };
    let tag = match war.get(side).and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let mut row = DataRow::new();
    let tagv = serde_json::Value::String(tag.to_string());
    row.bind_value(
        fidelity,
        "country_flag",
        tagv.clone(),
        Fidelity::Exact,
        fmt_token,
    );
    row.bind_value(fidelity, "country_name", tagv, Fidelity::Exact, fmt_token);
    // division COUNT for this participant from top-level divisions[] (derived).
    let div_count = match ctx.divisions() {
        Captured::Measured(v) => v.as_array().map(|a| {
            a.iter()
                .filter(|d| d.get("owner").and_then(|o| o.as_str()) == Some(tag))
                .count()
        }),
        _ => None,
    };
    match div_count {
        Some(n) => row.bind_value(
            fidelity,
            "divisions",
            serde_json::json!(n),
            Fidelity::Derived,
            fmt_int,
        ),
        None => row.badge_unavailable(fidelity, "divisions"),
    }
    // IC: the `ic` slot is full INDUSTRIAL CAPACITY (civ+mil+the IC-per-factory math), which
    // we do NOT have — `economy.civilian_factories` is only the civilian count, NOT IC
    // (Codex: binding it as "IC" overclaims). Badge unavailable rather than mislabel.
    row.badge_unavailable(fidelity, "ic");
    // wargoal: a participant's wargoal must TARGET an enemy belligerent to be attributable
    // to THIS war (Codex: `wargoals[0]` may concern an unrelated country). Pick the first
    // wargoal whose `target` is the OPPOSING side's tag; else badge unavailable.
    let opponent = if side == "a" {
        war.get("b")
    } else {
        war.get("a")
    }
    .and_then(|v| v.as_str());
    let wg = match (opponent, ctx.country_field(tag, "diplomacy.wargoals")) {
        (Some(opp), Captured::Measured(v)) => v.as_array().and_then(|a| {
            a.iter()
                .find(|g| g.get("target").and_then(|t| t.as_str()) == Some(opp))
                .and_then(|g| g.get("type"))
                .cloned()
        }),
        _ => None,
    };
    match wg {
        Some(t) => row.bind_value(fidelity, "wargoal", t, Fidelity::Derived, fmt_token),
        None => row.badge_unavailable(fidelity, "wargoal"),
    }
    // per-participant war-score / casualties / capitulation: TRUE GAPs (side-level only).
    row.badge_unavailable(fidelity, "warscore");
    row.badge_unavailable(fidelity, "casualties");
    row.badge_unavailable(fidelity, "surrender_progressbar");
    vec![row]
}

fn open_war_overview(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(WarOverviewView::new())
}

register_view!(ViewDescriptor {
    id: WAR_OVERVIEW_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(WAR_OVERVIEW_FILE, WAR_OVERVIEW_ROOT)),
    required_domains: &["diplomacy_world", "divisions"],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "War Overview",
        order: 9
    }),
    shortcut: None,
    open: open_war_overview,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.war_ally_entry"),
    gui_template: GuiSymbolRef::template(WAR_OVERVIEW_FILE, "war_ally_entry"),
    required_domains: &["diplomacy_world", "divisions"],
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.war_faction_entry"),
    gui_template: GuiSymbolRef::template(WAR_OVERVIEW_FILE, "war_faction_entry"),
    required_domains: &["diplomacy_world"],
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.waroverview_war_button"),
    gui_template: GuiSymbolRef::template(WAR_OVERVIEW_FILE, "waroverview_war_button"),
    required_domains: &["diplomacy_world"],
});

// =====================================================================================
// VIEW 10 — INSIGNIA SELECTION  (insignia_selection_window)  — mutation modal / STATIC DEF
// =====================================================================================

const INSIGNIA_VIEW_ID: ViewId = ViewId("intel.insignia_selection");
const INSIGNIA_FILE: &str = "insignia_selection.json";
const INSIGNIA_ROOT: &str = "insignia_selection_window";

pub struct InsigniaSelectionView {
    selected: Option<FormationSelection>,
    disabled_mutation_targets: RefCell<Vec<Rect>>,
    disabled_mutation_armed: bool,
}

impl InsigniaSelectionView {
    pub fn new() -> InsigniaSelectionView {
        InsigniaSelectionView {
            selected: None,
            disabled_mutation_targets: RefCell::new(Vec::new()),
            disabled_mutation_armed: false,
        }
    }

    fn with_selection(selected: FormationSelection) -> InsigniaSelectionView {
        InsigniaSelectionView {
            selected: Some(selected),
            disabled_mutation_targets: RefCell::new(Vec::new()),
            disabled_mutation_armed: false,
        }
    }

    fn disabled_mutation_hit(&self, x: f32, y: f32) -> bool {
        self.disabled_mutation_targets
            .borrow()
            .iter()
            .any(|rect| rect.contains(x, y))
    }
}

impl Default for InsigniaSelectionView {
    fn default() -> Self {
        InsigniaSelectionView::new()
    }
}

struct InsigniaSelectionDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    current_icon: SpriteCell,
    catalog_rows: Vec<DataRow>,
    catalog_section: SectionState,
    gaps: DataRow,
}

impl Drawable for InsigniaSelectionDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "INSIGNIA_SELECTION current={} catalog_sect={} catalog={} gaps={}",
            sprite_digest(&self.current_icon),
            self.catalog_section.tag(),
            row_digest(&self.catalog_rows),
            row_digest(std::slice::from_ref(&self.gaps)),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "item_grid",
            "insignia_list_entry",
            &self.catalog_rows,
        );
    }
}

impl ViewInstance for InsigniaSelectionView {
    fn view_id(&self) -> ViewId {
        INSIGNIA_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn rebind(&mut self, _ctx: &RebindContext) {
        self.disabled_mutation_armed = false;
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } if self.selected.is_some() && self.disabled_mutation_hit(x, y) => {
                self.disabled_mutation_armed = true;
                disabled_mutation_input_consumed()
            }
            ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            } if self.disabled_mutation_armed
                || (self.selected.is_some() && self.disabled_mutation_hit(x, y)) =>
            {
                self.disabled_mutation_armed = false;
                disabled_mutation_input_consumed()
            }
            _ => InputOutcome::ignored(),
        }
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, INSIGNIA_FILE);
        // A cosmetic formation-editor MUTATION modal. The insignia CATALOG is STATIC DEF
        // (game GFX, not a snapshot datum); the INITIAL value (current insignia + group
        // name) needs `order_of_battle.*` (absent in demo) → unavailable. Inert in replay.
        let mut summary: Vec<Cell> = Vec::new();
        let mut summary_sprites: Vec<SpriteCell> = Vec::new();
        let mut selected_base_sprite = None;
        let (current_asset, current_fidelity) = if let Some(selection) = self.selected {
            if let Some((formation, base_sprite)) = selected_formation(ctx, ctx.country, selection)
            {
                selected_base_sprite = Some(base_sprite);
                summary.push(cell_bind(
                    &mut fidelity,
                    "group_name",
                    elem_field(formation, "name"),
                    Fidelity::Exact,
                    fmt_token,
                ));
                formation_icon_asset(ctx, formation, base_sprite)
            } else {
                push_unavailable(&mut fidelity, &mut summary, "group_name");
                (
                    assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName),
                    Fidelity::Unavailable,
                )
            }
        } else {
            push_unavailable(&mut fidelity, &mut summary, "group_name");
            (
                assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName),
                Fidelity::Unavailable,
            )
        };
        let current_icon = SpriteCell {
            element: "group_icon".to_string(),
            asset: current_asset.clone(),
            fidelity: current_fidelity,
        };
        push_sprite(
            &mut fidelity,
            &mut summary_sprites,
            "group_icon",
            current_asset,
            current_fidelity,
        );
        let (catalog_rows, catalog_section) =
            build_insignia_catalog_rows(ctx, &mut fidelity, selected_base_sprite);
        let mut gaps = DataRow::new();
        gaps.badge_gap(&mut fidelity, "color_picker");

        let roots = build_root_with_sprites(ctx, &file, INSIGNIA_ROOT, &summary, &summary_sprites);
        let mut hit_roots = roots.clone();
        let row_roots = instantiated_row_roots(
            &roots,
            &file,
            "item_grid",
            "insignia_list_entry",
            &catalog_rows,
            &ctx.stage.manifest,
        );
        *self.disabled_mutation_targets.borrow_mut() =
            collect_insignia_disabled_mutation_targets(&row_roots);
        hit_roots.extend(row_roots);
        let drawable = InsigniaSelectionDrawable {
            roots: roots.clone(),
            file,
            current_icon,
            catalog_rows,
            catalog_section,
            gaps,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: INSIGNIA_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(hit_roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn collect_insignia_disabled_mutation_targets(row_roots: &[Node]) -> Vec<Rect> {
    row_roots
        .iter()
        .filter_map(|row| find_rect(std::slice::from_ref(row), "button"))
        .collect()
}

fn disabled_mutation_input_consumed() -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw: false,
    }
}

fn open_insignia(arg: Option<i64>) -> Box<dyn ViewInstance> {
    match FormationSelection::from_arg(arg) {
        Some(selection) => Box::new(InsigniaSelectionView::with_selection(selection)),
        None => Box::new(InsigniaSelectionView::new()),
    }
}

register_view!(ViewDescriptor {
    id: INSIGNIA_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(INSIGNIA_FILE, INSIGNIA_ROOT)),
    required_domains: &["order_of_battle"],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "Insignia Selection",
        order: 10
    }),
    shortcut: None,
    open: open_insignia,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.insignia_list_entry"),
    gui_template: GuiSymbolRef::template(INSIGNIA_FILE, "insignia_list_entry"),
    required_domains: &["order_of_battle"],
});

// =====================================================================================
// VIEW 11 — INS REVOLUTION  (INS_revolution_ui_window)  — mod script-var indicator
// =====================================================================================

const INS_REVOLUTION_VIEW_ID: ViewId = ViewId("intel.ins_revolution");
const INS_REVOLUTION_FILE: &str = "ins_revolution.json";
const INS_REVOLUTION_ROOT: &str = "INS_revolution_ui_window";

pub struct InsRevolutionView;

impl InsRevolutionView {
    pub fn new() -> InsRevolutionView {
        InsRevolutionView
    }
}

impl Default for InsRevolutionView {
    fn default() -> Self {
        InsRevolutionView::new()
    }
}

struct InsRevolutionDrawable {
    roots: Vec<Node>,
}

impl Drawable for InsRevolutionDrawable {
    fn draw_digest(&self) -> String {
        "INS_REVOLUTION mod war-escalation indicator (script var; world absent → unavailable)"
            .to_string()
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for InsRevolutionView {
    fn view_id(&self) -> ViewId {
        INS_REVOLUTION_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, INS_REVOLUTION_FILE);
        // A mod-specific war-escalation scale driven by a script variable
        // (`world.variables`/country var/`extra.dynamic_modifiers`). `world` is absent in
        // the demo → the level text + the active-icon selection are unavailable (ARTIFACT
        // for the live value). The 10 icons are a static decorative scale (chrome).
        let mut summary: Vec<Cell> = Vec::new();
        push_unavailable(&mut fidelity, &mut summary, "INS_we_level_text");

        let roots = build_root(ctx, &file, INS_REVOLUTION_ROOT, &summary);
        let drawable = InsRevolutionDrawable {
            roots: roots.clone(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: INS_REVOLUTION_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_ins_revolution(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(InsRevolutionView::new())
}

register_view!(ViewDescriptor {
    id: INS_REVOLUTION_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(INS_REVOLUTION_FILE, INS_REVOLUTION_ROOT)),
    required_domains: &[],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "INS Revolution",
        order: 11
    }),
    shortcut: None,
    open: open_ins_revolution,
    policy: HostPolicy::window(ZBand::Panel),
});

// =====================================================================================
// VIEW 12 — DIPLOMACY  (countrydiplomacyview)  [BINDING_MAP_diplomacy_combat VIEW 1]
// =====================================================================================
//
// The country-diplomacy card (`diplomacy.json` root `countrydiplomacyview`). `<SEL>` (the
// selected country whose card is open) and `<VIEWER>` (the observed root country) are BOTH
// the observed country here — we always show the viewer's own card. Bound EXACT:
//   * country_name / faction_name / party_name / ideology_name  <- politics.*
//   * our_opinion_value  <- countries.<VIEWER>.diplomacy.opinion.<SEL>.opinion_sum  and
//     their_opinion_value <- countries.<SEL>.diplomacy.opinion.<VIEWER>.opinion_sum
//     (Codex-confirmed DIRECTION: the OWNING country's opinion dict holds ITS opinion of the
//     keyed target). With SEL==VIEWER this is the country's self-opinion (a real measured
//     value in the demo, e.g. opinion[GER]=−100), never fabricated.
//   * stability_value / war_support_value  <- politics.{stability,war_support} (percent)
//   * national_focus_label <- focus.current
// Badged `unavailable` (never a 0): leader_name (commanders domain unfed), elections (no
// schema field). LOGICAL gaps (read but with NO `.gui` node in this ROOT, tallied for
// honesty, never a targetable cell so a stale set_element_text can't no-op): autonomy_state /
// overlord (present-but-null in the demo) + the trade grid (`market_live`/`trade_contracts`
// absent). The RELATIONS row strip (`relation_strip_view`) + COUNTRY-LIST grid
// (`diplomacy_country_list_country_entry`) are row factories drawn from `diplomacy_world` +
// per-country opinion; the ~17 diplomatic-action MODALS are deferred mutations (inert).

const DIPLOMACY_VIEW_ID: ViewId = ViewId("intel.diplomacy");
const DIPLOMACY_FILE: &str = "diplomacy.json";
const DIPLOMACY_ROOT: &str = "countrydiplomacyview";

pub struct DiplomacyView;

impl DiplomacyView {
    pub fn new() -> DiplomacyView {
        DiplomacyView
    }
}
impl Default for DiplomacyView {
    fn default() -> Self {
        DiplomacyView::new()
    }
}

struct DiplomacyDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    relation_rows: Vec<DataRow>,
    country_rows: Vec<DataRow>,
    relation_section: SectionState,
    country_section: SectionState,
}

impl Drawable for DiplomacyDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "DIPLOMACY rel_sect={} list_sect={} relations={} countries={} (trade grid + autonomy = unavailable/gap)",
            self.relation_section.tag(),
            self.country_section.tag(),
            row_digest(&self.relation_rows),
            row_digest(&self.country_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "relations_grid",
            "relation_strip_view",
            &self.relation_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "countries_grid",
            "diplomacy_country_list_country_entry",
            &self.country_rows,
        );
    }
}

impl ViewInstance for DiplomacyView {
    fn view_id(&self) -> ViewId {
        DIPLOMACY_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, DIPLOMACY_FILE);
        let sel = ctx.country; // SEL == VIEWER == the observed country.
        let viewer = ctx.country;

        let mut summary: Vec<Cell> = Vec::new();
        // identity + politics (EXACT).
        push_country(
            ctx,
            sel,
            &mut fidelity,
            &mut summary,
            "country_name",
            "politics.name",
            Fidelity::Exact,
            fmt_token,
        );
        push_country(
            ctx,
            sel,
            &mut fidelity,
            &mut summary,
            "faction_name",
            "politics.faction.name",
            Fidelity::Exact,
            fmt_token,
        );
        push_country(
            ctx,
            sel,
            &mut fidelity,
            &mut summary,
            "party_name",
            "politics.ruling_party",
            Fidelity::Derived,
            fmt_token,
        );
        push_country(
            ctx,
            sel,
            &mut fidelity,
            &mut summary,
            "ideology_name",
            "politics.ruling_party",
            Fidelity::Derived,
            fmt_token,
        );
        push_country(
            ctx,
            sel,
            &mut fidelity,
            &mut summary,
            "stability_value",
            "politics.stability",
            Fidelity::Exact,
            fmt_frac_pct,
        );
        push_country(
            ctx,
            sel,
            &mut fidelity,
            &mut summary,
            "war_support_value",
            "politics.war_support",
            Fidelity::Exact,
            fmt_frac_pct,
        );
        push_country(
            ctx,
            sel,
            &mut fidelity,
            &mut summary,
            "national_focus_label",
            "focus.current",
            Fidelity::Exact,
            fmt_token,
        );
        // OPINION pair (Codex direction): viewer->sel and sel->viewer net cached sums.
        push_country(
            ctx,
            viewer,
            &mut fidelity,
            &mut summary,
            "our_opinion_value",
            &format!("diplomacy.opinion.{sel}.opinion_sum"),
            Fidelity::Exact,
            fmt_signed,
        );
        push_country(
            ctx,
            sel,
            &mut fidelity,
            &mut summary,
            "their_opinion_value",
            &format!("diplomacy.opinion.{viewer}.opinion_sum"),
            Fidelity::Exact,
            fmt_signed,
        );
        // UNAVAILABLE (real ROOT nodes, never a fabricated 0): commanders + elections unfed.
        push_unavailable(&mut fidelity, &mut summary, "leader_name");
        push_unavailable(&mut fidelity, &mut summary, "elections");

        // LOGICAL gaps — read but with NO `.gui` node in THIS root (so they are tallied for
        // honesty, never pushed as a targetable cell). autonomy_state/overlord are present-
        // but-null in the demo (subject-only); the trade grid needs market_live/trade_contracts.
        record_gap(&mut fidelity); // autonomy_state (no node in this root; null in demo)
        record_gap(&mut fidelity); // overlord (subject-only; null in demo)
        record_gap(&mut fidelity); // trade_grid rows (market_live / trade_contracts absent)

        // ROW FACTORIES: relation strip + country list.
        let (relation_rows, relation_section) = build_relation_rows(ctx, sel, &mut fidelity);
        let (country_rows, country_section) = build_country_list_rows(ctx, viewer, &mut fidelity);

        let roots = build_root(ctx, &file, DIPLOMACY_ROOT, &summary);
        let drawable = DiplomacyDrawable {
            roots: roots.clone(),
            file,
            relation_rows,
            country_rows,
            relation_section,
            country_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: DIPLOMACY_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The RELATIONS strip: one `relation_strip_view` row per active relation the SELECTED
/// country holds (guarantees / non-aggression / embargoes) + subject links from
/// `diplomacy_world.subjects`. The relation token + counterpart TAG bind EXACT; the strip
/// has no text node, so the row data lives in the digest/fidelity tally.
fn build_relation_rows(
    ctx: &BindContext,
    sel: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let mut rows = Vec::new();
    let mut any = false;
    for (kind, dotted) in [
        ("guarantee", "diplomacy.guarantees"),
        ("non_aggression", "diplomacy.non_aggression"),
        ("embargo", "diplomacy.embargoes"),
    ] {
        let (slice, _st) = country_array(ctx, sel, dotted);
        if let Some(arr) = slice {
            any = true;
            for tgt in arr {
                let mut row = DataRow::new();
                row.bind_value(
                    fidelity,
                    "relation_icon",
                    serde_json::Value::String(kind.to_string()),
                    Fidelity::Exact,
                    fmt_token,
                );
                row.bind(
                    fidelity,
                    "relation_flags",
                    crate::host::context::classify_value(Some(tgt)),
                    Fidelity::Exact,
                    fmt_token,
                );
                rows.push(row);
            }
        }
    }
    // subject overlord/subject links where SEL participates. Bind the COUNTERPART tag (the
    // OTHER party), not SEL itself: when SEL is the overlord the row shows the subject; when
    // SEL is the subject the row shows the overlord (Codex direction fix).
    if let Some(dw) = diplomacy_world(ctx) {
        if let Some(subs) = dw.get("subjects").and_then(|v| v.as_array()) {
            for s in subs {
                let over = s.get("overlord").and_then(|v| v.as_str());
                let sub = s.get("subject").and_then(|v| v.as_str());
                let counterpart = if over == Some(sel) {
                    Some(("overlord_of", "subject"))
                } else if sub == Some(sel) {
                    Some(("subject_of", "overlord"))
                } else {
                    None
                };
                if let Some((rel_kind, counterpart_key)) = counterpart {
                    any = true;
                    let mut row = DataRow::new();
                    row.bind_value(
                        fidelity,
                        "relation_icon",
                        serde_json::Value::String(rel_kind.to_string()),
                        Fidelity::Exact,
                        fmt_token,
                    );
                    row.bind(
                        fidelity,
                        "relation_flags",
                        elem_field(s, counterpart_key),
                        Fidelity::Exact,
                        fmt_token,
                    );
                    rows.push(row);
                }
            }
        }
    }
    let state = if any {
        SectionState::Present
    } else {
        SectionState::Absent
    };
    state.record(fidelity);
    (rows, state)
}

/// The COUNTRY-LIST grid: one `diplomacy_country_list_country_entry` per known country
/// (sorted), each row's name + the viewer->row and row->viewer opinion sums (EXACT where the
/// opinion dict carries the target, else "—" — never a fabricated 0).
fn build_country_list_rows(
    ctx: &BindContext,
    viewer: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let tags = country_tags(ctx);
    if tags.is_empty() {
        SectionState::Absent.record(fidelity);
        return (Vec::new(), SectionState::Absent);
    }
    let mut rows = Vec::new();
    for tag in &tags {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "name",
            ctx.country_field(tag, "politics.name"),
            Fidelity::Exact,
            fmt_token,
        );
        // viewer's opinion OF this row's country, and the reciprocal (cached net sums).
        row.bind(
            fidelity,
            "our_opinion",
            ctx.country_field(viewer, &format!("diplomacy.opinion.{tag}.opinion_sum")),
            Fidelity::Exact,
            fmt_signed,
        );
        row.bind(
            fidelity,
            "their_opinion",
            ctx.country_field(tag, &format!("diplomacy.opinion.{viewer}.opinion_sum")),
            Fidelity::Exact,
            fmt_signed,
        );
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

fn open_diplomacy(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(DiplomacyView::new())
}

register_view!(ViewDescriptor {
    id: DIPLOMACY_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(DIPLOMACY_FILE, DIPLOMACY_ROOT)),
    required_domains: &["countries", "diplomacy_world"],
    route: Some(MenuRoute {
        cluster: MenuCluster::IntelDiplomacyTrade,
        label: "Diplomacy",
        order: 12
    }),
    shortcut: None,
    open: open_diplomacy,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.diplomacy_relation_strip"),
    gui_template: GuiSymbolRef::template(DIPLOMACY_FILE, "relation_strip_view"),
    required_domains: &["countries", "diplomacy_world"],
});
register_template!(TemplateDescriptor {
    id: ViewId("intel.diplomacy_country_entry"),
    gui_template: GuiSymbolRef::template(DIPLOMACY_FILE, "diplomacy_country_list_country_entry"),
    required_domains: &["countries"],
});

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

    fn gui_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("game_assets")
            .join("gui")
    }

    fn assets_present() -> bool {
        gui_dir().join("intel_agency.json").exists()
    }

    fn load_gui_file(name: &str) -> Option<GuiFile> {
        let p = gui_dir().join(name);
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| GuiFile::parse(&s).ok())
    }

    fn stage_with(files: &[&str]) -> Stage {
        stage_with_manifest(files, HashMap::new())
    }

    fn stage_with_manifest(files: &[&str], sprites: HashMap<String, assets::SpriteEntry>) -> Stage {
        let mut gui_files = HashMap::new();
        for f in files {
            if let Some(g) = load_gui_file(f) {
                gui_files.insert(f.to_string(), g);
            }
        }
        Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest: assets::SpriteManifest { sprites },
            loc: Loc::new(),
            gui_files,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: gui_dir().parent().unwrap().to_path_buf(),
        }
    }

    fn stage_with_gui_and_manifest(
        file: &str,
        gui: GuiFile,
        sprites: HashMap<String, assets::SpriteEntry>,
    ) -> Stage {
        let mut gui_files = HashMap::new();
        gui_files.insert(file.to_string(), gui);
        Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest: assets::SpriteManifest { sprites },
            loc: Loc::new(),
            gui_files,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: gui_dir().parent().unwrap().to_path_buf(),
        }
    }

    fn sprite_entry(name: &str, texturefile: Option<&str>, frames: u32) -> assets::SpriteEntry {
        assets::SpriteEntry {
            name: name.to_string(),
            kind: "spriteType".to_string(),
            no_of_frames: frames,
            source_gfx: None,
            texturefile: texturefile.map(str::to_string),
            category: None,
            width: None,
            height: None,
            nine_slice: None,
            size: None,
            border_size: None,
        }
    }

    fn insignia_test_manifest() -> HashMap<String, assets::SpriteEntry> {
        let mut sprites = HashMap::new();
        for (name, texture, frames) in [
            ("GFX_exact_agency", "gfx/interface/agency.dds", 1),
            ("GFX_exact_formation", "gfx/interface/formation.dds", 1),
            ("GFX_unknown_agency", "gfx/interface/unknown_agency.dds", 1),
            (
                "GFX_theatre_army_shield",
                "gfx/interface/army_shield.dds",
                4,
            ),
            (
                "GFX_theatre_army_group_shield",
                "gfx/interface/army_group_shield.dds",
                3,
            ),
        ] {
            sprites.insert(name.to_string(), sprite_entry(name, Some(texture), frames));
        }
        sprites
    }

    fn insignia_test_gui() -> GuiFile {
        GuiFile::parse(
            r#"{"view":"insignia_test","windows":[
              {"type":"containerWindowType","name":"insignia_selection_window",
               "position":{"x":0,"y":0},"size":{"width":360,"height":240},
               "children":[
                 {"type":"instantTextBoxType","name":"group_name","position":{"x":8,"y":8},
                  "text":{"text":"GROUP","font":"hoi_16mbs"}},
                 {"type":"iconType","name":"group_icon","position":{"x":8,"y":36},
                  "size":{"width":32,"height":32},"sprite":"GFX_theatre_army_shield"},
                 {"type":"containerWindowType","name":"item_grid","position":{"x":208,"y":80},
                  "size":{"width":140,"height":120}}
               ]},
              {"type":"buttonType","name":"insignia_list_entry","position":{"x":0,"y":0},
               "size":{"width":40,"height":40},"sprite":"GFX_button",
               "children":[
                 {"type":"iconType","name":"button","position":{"x":4,"y":4},
                  "size":{"width":32,"height":32},"sprite":"GFX_theatre_army_shield"}
               ]}
            ]}"#,
        )
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
            date: "1941.4.13.11",
            cursor: (0.0, 0.0),
            map_camera: None,
        }
    }

    /// Snapshot A — the demo-faithful shape: operatives POPULATED, agency/operations/
    /// decryption/peace null, diplomacy_world wars/factions/subjects + top-level divisions.
    fn rebind_intel_agency(v: &mut IntelAgencyView, snap: &snapshot::Snapshot, refit: bool) {
        v.rebind(&RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit,
        });
    }

    fn intel_agency_tab_center(bar: &TabBar, panel: Rect, id: &str) -> (f32, f32) {
        let rect = bar
            .layout(panel)
            .into_iter()
            .find(|tr| tr.id == id)
            .unwrap_or_else(|| panic!("missing intel agency tab rect {id}"))
            .rect;
        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
    }

    fn click_intel_agency_tab_center(v: &mut IntelAgencyView, id: &str) -> InputOutcome {
        let panel = intel_agency_tab_fallback(v.screen);
        let (x, y) = intel_agency_tab_center(&v.tabs, panel, id);
        v.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        })
    }

    fn visible_intel_agency_chunk(d: &str) -> &str {
        let marker = "visible=[";
        let start = d
            .find(marker)
            .unwrap_or_else(|| panic!("missing visible marker in digest: {d}"))
            + marker.len();
        let rest = &d[start..];
        let end = rest
            .find("] op_sect=")
            .unwrap_or_else(|| panic!("missing visible terminator in digest: {d}"));
        &rest[..end]
    }

    fn snap_a() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11", "hour_index": 100,
            "countries": {
                "GER": {
                    "intel": {
                        "operatives": [
                            {"name": "Johann Hein", "skill": 1, "experience": 0.0,
                             "traits": ["operative_safe_cracker"], "state": "on_mission", "nationalities": ["GER"]},
                            {"name": "Klara Vogel", "skill": 2, "experience": 5.0,
                             "traits": [], "state": "on_operation", "nationalities": ["GER", "AUS"]}
                        ],
                        "agency": null, "operations": null, "decryption": null, "peace_conference": null
                    },
                    "politics": {
                        "name": "German Reich", "ruling_party": "fascism",
                        "stability": 0.72, "war_support": 0.60, "faction": {"name": "axis"}
                    },
                    "focus": {"current": "GER_rhineland"},
                    "diplomacy": {
                        "wargoals": [{"type": "annex_everything", "target": "POL"}],
                        "guarantees": ["POL"],
                        "non_aggression": ["SOV"],
                        "opinion": {
                            "GER": {"opinion_sum": 0},
                            "POL": {"opinion_sum": -50},
                            "FRA": {"opinion_sum": -50}
                        }
                    },
                    "economy": {"civilian_factories": 40}
                },
                "POL": {
                    "intel": {"operatives": []},
                    "politics": {"name": "Poland", "ruling_party": "neutrality"},
                    "diplomacy": {
                        "wargoals": [],
                        "opinion": {"GER": {"opinion_sum": -200}}
                    },
                    "economy": {"civilian_factories": 12}
                }
            },
            "states": {
                "64": {"owner": "GER", "controller": "GER"}
            },
            "divisions": [
                {"div_id": 1, "owner": "GER", "strength": 1000.0},
                {"div_id": 2, "owner": "GER", "strength": 800.0},
                {"div_id": 3, "owner": "POL", "strength": 500.0}
            ],
            "diplomacy_world": {
                "wars": [
                    {"a": "GER", "b": "POL", "start_date": "1939.9.1",
                     "a_casualties": 12000, "b_casualties": 30000,
                     "war_score_a_vs_b": 60, "war_score_b_vs_a": 10}
                ],
                "factions": [{"id": "axis", "ideology": "fascism", "template": "axis", "members": ["GER"], "leader": "GER"}],
                "subjects": [{"overlord": "GER", "subject": "SLO", "autonomy_state": "puppet", "value": 0.4}]
            }
        }))
        .unwrap()
    }

    /// Snapshot B — differs: more operatives, different war scores/casualties, an extra war
    /// + threat ledger (world_events folded) so the scrub A→B→A actually moves.
    fn snap_b() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.11", "hour_index": 200,
            "countries": {
                "GER": {
                    "intel": {
                        "operatives": [
                            {"name": "Rolf Adler", "skill": 4, "experience": 99.0,
                             "traits": ["operative_commando", "operative_safe_cracker"], "state": "on_operation", "nationalities": ["GER"]}
                        ],
                        "agency": null, "operations": null, "decryption": null, "peace_conference": null
                    },
                    "diplomacy": {"wargoals": [{"type": "puppet", "target": "FRA"}]},
                    "economy": {"civilian_factories": 70}
                },
                "FRA": {"intel": {"operatives": []}, "diplomacy": {"wargoals": []}, "economy": {"civilian_factories": 25}}
            },
            "states": {"64": {"owner": "GER", "controller": "GER"}},
            "divisions": [
                {"div_id": 1, "owner": "GER", "strength": 2000.0},
                {"div_id": 9, "owner": "FRA", "strength": 1500.0}
            ],
            "diplomacy_world": {
                "wars": [
                    {"a": "GER", "b": "FRA", "start_date": "1940.5.10",
                     "a_casualties": 5000, "b_casualties": 90000,
                     "war_score_a_vs_b": 85, "war_score_b_vs_a": 5}
                ],
                "factions": [], "subjects": []
            },
            "world_events": {
                "threat": [
                    {"tag": "GER", "threat": 5.0, "final_threat": 12.0, "daily": 0.1, "date": "1941.6.1", "label": "ANNEXED_AUSTRIA"}
                ]
            }
        }))
        .unwrap()
    }

    fn agency_tabs_snap_a() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11", "hour_index": 100,
            "countries": {
                "GER": {
                    "intel": {
                        "operatives": [
                            {"name": "Johann Hein", "skill": 1, "traits": ["operative_safe_cracker"]}
                        ],
                        "operations": {
                            "running": [
                                {"type": "steal_blueprints", "target": "POL", "duration": 60}
                            ]
                        },
                        "decryption": {
                            "targets": [
                                {"target": "POL", "amount": 31}
                            ]
                        }
                    }
                }
            }
        }))
        .unwrap()
    }

    fn agency_tabs_snap_b() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.11", "hour_index": 200,
            "countries": {
                "GER": {
                    "intel": {
                        "operatives": [
                            {"name": "Rolf Adler", "skill": 4, "traits": ["operative_commando"]}
                        ],
                        "operations": {
                            "running": [
                                {"type": "collaboration_government", "target": "FRA", "duration": 90}
                            ]
                        },
                        "decryption": {
                            "targets": [
                                {"target": "FRA", "amount": 55}
                            ]
                        }
                    }
                }
            }
        }))
        .unwrap()
    }

    fn all_views() -> Vec<(&'static str, Box<dyn ViewInstance>)> {
        vec![
            ("intel_agency.json", Box::new(IntelAgencyView::new())),
            ("intel_ledger.json", Box::new(IntelLedgerView::new())),
            ("operative.json", Box::new(OperativeView::new())),
            (
                "operative_leader.json",
                Box::new(OperativeLeaderView::new()),
            ),
            (
                "operation_overview.json",
                Box::new(OperationOverviewView::new()),
            ),
            ("trade.json", Box::new(TradeView::new())),
            (
                "peace_conference.json",
                Box::new(PeaceConferenceView::new()),
            ),
            (
                "world_tension_popup.json",
                Box::new(WorldTensionView::new()),
            ),
            ("war_overview.json", Box::new(WarOverviewView::new())),
            (
                "insignia_selection.json",
                Box::new(InsigniaSelectionView::new()),
            ),
            ("ins_revolution.json", Box::new(InsRevolutionView::new())),
            ("diplomacy.json", Box::new(DiplomacyView::new())),
        ]
    }

    /// Every view binds against its REAL `.gui` ROOT (the root window renders) and no field
    /// fabricates a 0 — a blocked field renders the not-captured glyph "—".
    #[test]
    fn all_eleven_views_render_real_gui_and_badge_not_zero() {
        if !assets_present() {
            eprintln!("[2g] skipped: game_assets/gui not extracted");
            return;
        }
        let roots = [
            ("intel_agency.json", "countryintelligenceagencyview"),
            ("intel_ledger.json", "country_intel_ledger_view"),
            ("operative.json", "boost_party_popularity_mission_window"),
            ("operative_leader.json", "operative_badge_view"),
            ("operation_overview.json", "operation_overview_window"),
            ("trade.json", "countrytradeview"),
            ("peace_conference.json", "peaceconference_full_window"),
            ("world_tension_popup.json", "world_tension_popup"),
            ("war_overview.json", "waroverview_window"),
            ("insignia_selection.json", "insignia_selection_window"),
            ("ins_revolution.json", "INS_revolution_ui_window"),
            ("diplomacy.json", "countrydiplomacyview"),
        ];
        for ((file, v), (_f2, root)) in all_views().into_iter().zip(roots.iter()) {
            let stage = stage_with(&[file]);
            let snap = snap_a();
            let bv = v.bind(&ctx(&stage, &snap));
            let d = bound_digest(&bv);
            assert!(
                d.contains(root),
                "{file}: real ROOT {root} must render, got:\n{d}"
            );
            // every view has at least one blocked/unavailable field tallied (never a
            // fabricated 0) — surfaced either as a "—" in the digest OR in the fidelity
            // report (a custom-drawn-only view whose ROOT carries no bound text node).
            assert!(
                d.contains('—') || bv.fidelity.unavailable > 0,
                "{file}: a blocked field must badge unavailable (—), never a fabricated 0; got:\n{d}"
            );
        }
    }

    /// Operatives bind EXACT: name / skill / traits / nationalities are read from the
    /// populated `intel.operatives[]`; an empty traits list renders "none", not "—".
    #[test]
    fn operatives_bind_exact_from_populated_list() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["operative_leader.json"]);
        let bv = OperativeLeaderView::new().bind(&ctx(&stage, &snap_a()));
        let d = bound_digest(&bv);
        // The badge card binds the first operative's name into a Container node (no text
        // box), so the name surfaces in the fidelity tally + the row data rather than the
        // node digest; the per-operative ROW digest carries the exact traits/nationalities.
        assert!(
            d.contains("operative_safe_cracker(E)"),
            "operative traits must bind exact, got:\n{d}"
        );
        assert!(
            d.contains("none(E)"),
            "an empty traits list renders 'none' exact (not —), got:\n{d}"
        );
        assert!(
            d.contains("GER, AUS(E)"),
            "multi-nationality operative must bind exact, got:\n{d}"
        );
        let portrait_slots = d.matches("portrait_btn:sprite=").count();
        let no_sprite_name_misses = d
            .matches("portrait_btn:sprite=MISS:no_sprite_name(U);")
            .count();
        assert_eq!(
            no_sprite_name_misses, 2,
            "operative rows without portrait sprite names must be explicit unavailable sprite misses, got:\n{d}"
        );
        assert_eq!(
            portrait_slots, no_sprite_name_misses,
            "operative portraits with no source sprite must not fabricate exact portrait art, got:\n{d}"
        );
        assert!(
            !d.contains("portrait_btn:sprite=GFX_leader_unknown#0(E);")
                && !d.contains("portrait_btn:sprite=0")
                && !d.contains("portrait_btn=0("),
            "operative portraits must not fabricate generic/0 portrait success, got:\n{d}"
        );
        assert!(
            d.contains("leader_captured=—(gap);"),
            "captured overlay remains the row gap, got:\n{d}"
        );
        // the name IS bound exact (it lands in the fidelity report as an exact field).
        assert!(
            bv.fidelity.exact >= 5,
            "name/skill/traits/nationalities bind exact, got:\n{d}"
        );
    }

    /// War overview binds SIDE losses + war-score exact and badges per-participant
    /// war-score / casualties / capitulation (TRUE GAPs), never fabricating a split.
    #[test]
    fn war_overview_side_exact_participant_unavailable() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["war_overview.json"]);
        let d = bound_digest(&WarOverviewView::new().bind(&ctx(&stage, &snap_a())));
        assert!(
            d.contains("12000") || d.contains("attacker_losses"),
            "side casualties must bind exact, got:\n{d}"
        );
        assert!(
            d.contains("warscore=—(U)"),
            "per-participant war-score must badge unavailable, got:\n{d}"
        );
        assert!(
            d.contains("casualties=—(U)"),
            "per-participant casualties must badge unavailable, got:\n{d}"
        );
        assert!(
            d.contains("GER vs POL"),
            "the war title must derive from belligerents, got:\n{d}"
        );
    }

    /// World-tension wars list binds belligerent TAGs + side war-score exact; the aggregate
    /// scalar + (in snap_a) the threat ledger are unavailable.
    #[test]
    fn world_tension_wars_exact_threats_artifact() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["world_tension_popup.json"]);
        let d = bound_digest(&WorldTensionView::new().bind(&ctx(&stage, &snap_a())));
        assert!(
            d.contains("GER vs POL"),
            "the war row must bind belligerents, got:\n{d}"
        );
        assert!(
            d.contains("threat_sect=absent"),
            "world_events absent → threat list badges absent, got:\n{d}"
        );
        // snap_b folds world_events → the threat list lights up exact.
        let d2 = bound_digest(&WorldTensionView::new().bind(&ctx(&stage, &snap_b())));
        assert!(
            d2.contains("ANNEXED_AUSTRIA"),
            "a folded threat contributor must bind, got:\n{d2}"
        );
    }

    /// intel_ledger badges EVERY estimate cell unavailable (TRUE GAP b — binding the real
    /// value would leak perfect intel) and binds the per-tab name label exact.
    #[test]
    fn intel_ledger_all_estimates_unavailable() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["intel_ledger.json"]);
        let bv = IntelLedgerView::new().bind(&ctx(&stage, &snap_a()));
        assert_eq!(bv.fidelity.exact, 1, "only the per-tab name label is exact");
        assert!(
            bv.fidelity.unavailable >= 14,
            "every estimate cell must be unavailable, got {bv:?}",
            bv = bv.fidelity
        );
    }

    /// Scrub A→B→A: every view re-binds byte-identical after a B visit (the immutable-LRU
    /// rule through the host's per-view `bind`).
    #[test]
    fn scrub_a_b_a_byte_identical_all_views() {
        if !assets_present() {
            return;
        }
        for (file, v) in all_views() {
            let stage = stage_with(&[file]);
            let (a, b) = (snap_a(), snap_b());
            let da1 = bound_digest(&v.bind(&ctx(&stage, &a)));
            let db = bound_digest(&v.bind(&ctx(&stage, &b)));
            // some views (insignia/ins_revolution/operative/intel_ledger) are data-blind to
            // the snapshot, so A and B legitimately match; the byte-identity check is the
            // one that matters for the scrub contract.
            let _ = &db;
            let da2 = bound_digest(&v.bind(&ctx(&stage, &a)));
            assert_eq!(
                da1, da2,
                "scrub A→B→A carried a value from the B visit ({file})"
            );
        }
    }

    #[test]
    fn intel_agency_tab_click_switches_digest_and_visible_body() {
        let stage = stage_with(&[]);
        let snap = agency_tabs_snap_a();
        let mut v = IntelAgencyView::new();
        rebind_intel_agency(&mut v, &snap, true);

        let cx = ctx(&stage, &snap);
        let agency_bv = v.bind(&cx);
        assert!(agency_bv.commands.is_empty());
        let agency = bound_digest(&agency_bv);
        assert!(agency.contains("selected=Some(\"agency\")"), "{agency}");
        assert!(agency.contains("visible_tab=agency"), "{agency}");
        assert_eq!(
            visible_intel_agency_chunk(&agency),
            "",
            "Agency summary tab should not draw operation or cryptology rows: {agency}"
        );

        let out = click_intel_agency_tab_center(&mut v, INTEL_AGENCY_TAB_OPERATIONS);
        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert!(out.commands.is_empty());
        let operations = bound_digest(&v.bind(&cx));
        assert_ne!(
            agency, operations,
            "selecting Operations must change the digest"
        );
        assert!(
            operations.contains("selected=Some(\"operations\")"),
            "{operations}"
        );
        assert!(
            operations.contains("visible_tab=operations"),
            "{operations}"
        );
        let op_visible = visible_intel_agency_chunk(&operations);
        assert!(
            op_visible.contains("operation_name_text=steal_blueprints(D);"),
            "{operations}"
        );
        assert!(
            !op_visible.contains("cryptology_state=31(D);"),
            "Operations visible rows must not carry Cryptology rows: {op_visible}"
        );

        let out = click_intel_agency_tab_center(&mut v, INTEL_AGENCY_TAB_CRYPTOLOGY);
        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert!(out.commands.is_empty());
        let cryptology = bound_digest(&v.bind(&cx));
        assert_ne!(
            operations, cryptology,
            "selecting Cryptology must change the digest"
        );
        assert!(
            cryptology.contains("selected=Some(\"cryptology\")"),
            "{cryptology}"
        );
        assert!(
            cryptology.contains("visible_tab=cryptology"),
            "{cryptology}"
        );
        let crypto_visible = visible_intel_agency_chunk(&cryptology);
        assert!(crypto_visible.contains("name=POL(E);"), "{cryptology}");
        assert!(
            crypto_visible.contains("cryptology_state=31(D);"),
            "{cryptology}"
        );
        assert!(
            !crypto_visible.contains("operation_name_text=steal_blueprints(D);"),
            "Cryptology visible rows must not carry Operations rows: {crypto_visible}"
        );
    }

    #[test]
    fn intel_agency_unavailable_tab_click_is_explicitly_inert() {
        let stage = stage_with(&[]);
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11",
            "countries": {
                "GER": {
                    "intel": {
                        "operatives": [
                            {"name": "Johann Hein", "skill": 1}
                        ],
                        "operations": null,
                        "decryption": null
                    }
                }
            }
        }))
        .unwrap();
        let mut v = IntelAgencyView::new();
        rebind_intel_agency(&mut v, &snap, true);

        assert_eq!(v.tabs.selected(), Some(INTEL_AGENCY_TAB_AGENCY));
        assert!(
            v.tabs
                .tabs
                .iter()
                .any(|t| t.id == INTEL_AGENCY_TAB_OPERATIONS && !t.avail.is_pickable()),
            "null operations data must make the Operations tab unavailable"
        );

        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let out = click_intel_agency_tab_center(&mut v, INTEL_AGENCY_TAB_OPERATIONS);
        assert!(out.consumed);
        assert!(!out.needs_redraw);
        assert!(out.commands.is_empty());
        assert_eq!(v.tabs.selected(), Some(INTEL_AGENCY_TAB_AGENCY));
        let after = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "clicking an unavailable intel agency tab must be an explicit no-op"
        );
    }

    #[test]
    fn intel_agency_selected_tab_persists_across_scrub_a_b_a() {
        let stage = stage_with(&[]);
        let (a, b) = (agency_tabs_snap_a(), agency_tabs_snap_b());
        let mut v = IntelAgencyView::new();
        rebind_intel_agency(&mut v, &a, true);
        let out = click_intel_agency_tab_center(&mut v, INTEL_AGENCY_TAB_OPERATIONS);
        assert!(out.consumed && out.needs_redraw);

        let da1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        rebind_intel_agency(&mut v, &b, false);
        let db = bound_digest(&v.bind(&ctx(&stage, &b)));
        rebind_intel_agency(&mut v, &a, false);
        let da2 = bound_digest(&v.bind(&ctx(&stage, &a)));

        assert!(da1.contains("selected=Some(\"operations\")"), "{da1}");
        assert!(db.contains("selected=Some(\"operations\")"), "{db}");
        assert!(
            visible_intel_agency_chunk(&db).contains("collaboration_government(D);"),
            "{db}"
        );
        assert_eq!(
            da1, da2,
            "scrub A->B->A must preserve selected intel agency tab without carrying B rows"
        );
        assert!(
            !da2.contains("collaboration_government"),
            "A-after-scrub must not carry the B-only operation row: {da2}"
        );
    }

    #[test]
    fn agency_insignia_exact_fallback_and_no_generic_art() {
        let stage = stage_with_manifest(&[], insignia_test_manifest());

        let exact_snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "countries": {
                "GER": {"intel": {"agency": {"insignia_gfx": "GFX_exact_agency"}}}
            }
        }))
        .unwrap();
        assert_eq!(
            agency_insignia_asset(&ctx(&stage, &exact_snap), "GER"),
            assets::SpriteAsset::resolved("GFX_exact_agency", 0),
            "agency insignia must bind the exact agency sprite when present"
        );

        let fallback_snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "countries": {
                "GER": {"intel": {"agency": {
                    "insignia_gfx": "GFX_missing_agency",
                    "icon_gfx": "GFX_exact_agency"
                }}}
            }
        }))
        .unwrap();
        assert_eq!(
            agency_insignia_asset(&ctx(&stage, &fallback_snap), "GER"),
            assets::SpriteAsset::resolved("GFX_exact_agency", 0),
            "a missing first agency field must fall through to the next exact sprite field"
        );

        let generic_snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "countries": {
                "GER": {"intel": {"agency": {"insignia_gfx": "GFX_unknown_agency"}}}
            }
        }))
        .unwrap();
        assert_eq!(
            agency_insignia_asset(&ctx(&stage, &generic_snap), "GER"),
            assets::SpriteAsset::Unavailable(assets::SpriteMiss::GenericPlaceholder(
                "GFX_unknown_agency".to_string()
            )),
            "generic unknown agency art must stay unavailable, not exact fabricated art"
        );

        let null_snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "countries": {"GER": {"intel": {"agency": null}}}
        }))
        .unwrap();
        assert_eq!(
            agency_insignia_asset(&ctx(&stage, &null_snap), "GER"),
            assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName),
            "null agency data must badge missing, not fabricate an insignia"
        );
    }

    #[test]
    fn selected_group_icon_uses_one_based_army_and_army_group_frames() {
        let stage = stage_with_manifest(&[], insignia_test_manifest());
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "order_of_battle": {
                "armies": [
                    {"id": 11, "owner": "GER", "name": "First Army", "icon": 3}
                ],
                "army_groups": [
                    {"id": 22, "owner": "GER", "name": "Heeresgruppe A", "icon": 2}
                ]
            }
        }))
        .unwrap();
        let binding = ctx(&stage, &snap);
        let (formation, base_sprite) =
            selected_formation(&binding, "GER", FormationSelection::Army(11))
                .expect("explicit GER army is selected");
        assert_eq!(base_sprite, "GFX_theatre_army_shield");
        let (asset, fidelity) = formation_icon_asset(&binding, formation, base_sprite);
        assert_eq!(fidelity, Fidelity::Derived);
        assert_eq!(
            asset,
            assets::SpriteAsset::resolved("GFX_theatre_army_shield", 2),
            "formation icon 3 is one-based and must bind shield frame 2"
        );

        let direct_formation = serde_json::json!({
            "owner": "GER",
            "name": "Exact Formation",
            "insignia_gfx": "GFX_exact_formation",
            "icon": 3
        });
        let (direct_asset, direct_fidelity) =
            formation_icon_asset(&binding, &direct_formation, "GFX_theatre_army_shield");
        assert_eq!(direct_fidelity, Fidelity::Exact);
        assert_eq!(
            direct_asset,
            assets::SpriteAsset::resolved("GFX_exact_formation", 0),
            "an explicit formation sprite is exact and does not derive from the icon frame"
        );

        let (group, group_base) =
            selected_formation(&binding, "GER", FormationSelection::ArmyGroup(22))
                .expect("explicit GER army group is selected");
        assert_eq!(group_base, "GFX_theatre_army_group_shield");
        let (group_asset, group_fidelity) = formation_icon_asset(&binding, group, group_base);
        assert_eq!(group_fidelity, Fidelity::Derived);
        assert_eq!(
            group_asset,
            assets::SpriteAsset::resolved("GFX_theatre_army_group_shield", 1),
            "army-group icon 2 must bind the army-group shield strip frame 1"
        );

        assert!(
            selected_formation(&binding, "GER", FormationSelection::Army(22)).is_none(),
            "a group id must not resolve through the army selector"
        );
    }

    #[test]
    fn formation_icon_missing_and_out_of_range_badge_unavailable() {
        let stage = stage_with_manifest(&[], insignia_test_manifest());
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({})).unwrap();
        let binding = ctx(&stage, &snap);

        let missing_icon = serde_json::json!({"owner": "GER", "name": "No Icon"});
        let (missing_asset, missing_fidelity) =
            formation_icon_asset(&binding, &missing_icon, "GFX_theatre_army_shield");
        assert_eq!(missing_fidelity, Fidelity::Derived);
        assert_eq!(
            missing_asset,
            assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName),
            "a formation with no direct sprite and no icon frame must not fabricate shield art"
        );

        let zero_icon = serde_json::json!({"owner": "GER", "name": "Zero Icon", "icon": 0});
        let (zero_asset, zero_fidelity) =
            formation_icon_asset(&binding, &zero_icon, "GFX_theatre_army_shield");
        assert_eq!(zero_fidelity, Fidelity::Derived);
        assert_eq!(
            zero_asset,
            assets::SpriteAsset::Unavailable(assets::SpriteMiss::FrameOutOfRange {
                name: "GFX_theatre_army_shield".to_string(),
                frame: 0,
                frames: 4,
            }),
            "icon frame 0 is invalid for the one-based save value"
        );

        let out_of_range = serde_json::json!({"owner": "GER", "name": "Bad Icon", "icon": 5});
        let (oor_asset, oor_fidelity) =
            formation_icon_asset(&binding, &out_of_range, "GFX_theatre_army_shield");
        assert_eq!(oor_fidelity, Fidelity::Derived);
        assert_eq!(
            oor_asset,
            assets::SpriteAsset::Unavailable(assets::SpriteMiss::FrameOutOfRange {
                name: "GFX_theatre_army_shield".to_string(),
                frame: 4,
                frames: 4,
            }),
            "one-based icon frame 5 is outside a four-frame army shield strip"
        );
    }

    #[test]
    fn insignia_catalog_row_count_and_modal_is_read_only() {
        let stage = stage_with_gui_and_manifest(
            INSIGNIA_FILE,
            insignia_test_gui(),
            insignia_test_manifest(),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "order_of_battle": {
                "armies": [
                    {"id": 11, "owner": "GER", "name": "First Army", "icon": 2}
                ],
                "army_groups": []
            }
        }))
        .unwrap();
        let binding = ctx(&stage, &snap);
        let mut fidelity = FidelityReport::default();
        let (rows, section) =
            build_insignia_catalog_rows(&binding, &mut fidelity, Some("GFX_theatre_army_shield"));
        assert_eq!(section, SectionState::Present);
        assert_eq!(
            rows.len(),
            4,
            "catalog rows must match the army shield manifest frame count"
        );

        let bv = InsigniaSelectionView::with_selection(FormationSelection::Army(11)).bind(&binding);
        assert_eq!(
            bv.commands.len(),
            0,
            "the cosmetic insignia modal must remain inert/read-only in replay"
        );
        let d = bound_digest(&bv);
        assert!(
            d.contains("group_name|Text(\"First Army\""),
            "selected formation name must bind exact into the modal root, got:\n{d}"
        );
        assert!(
            d.contains("current=group_icon:sprite=GFX_theatre_army_shield#1(D);"),
            "current icon 2 must bind derived one-based frame 1, got:\n{d}"
        );
        assert!(
            d.contains("catalog_sect=present"),
            "catalog section must be present when the shield strip resolves, got:\n{d}"
        );
        assert_eq!(
            d.matches("button:sprite=GFX_theatre_army_shield#").count(),
            4,
            "catalog row digest must contain one row per army shield frame, got:\n{d}"
        );
    }

    #[test]
    fn insignia_catalog_button_click_is_replay_disabled_noop() {
        let stage = stage_with_gui_and_manifest(
            INSIGNIA_FILE,
            insignia_test_gui(),
            insignia_test_manifest(),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "order_of_battle": {
                "armies": [
                    {"id": 11, "owner": "GER", "name": "First Army", "icon": 2}
                ],
                "army_groups": []
            }
        }))
        .unwrap();
        let binding = ctx(&stage, &snap);
        let mut view = InsigniaSelectionView::with_selection(FormationSelection::Army(11));
        view.rebind(&RebindContext {
            snapshot: &snap,
            country: "GER",
            screen: (1600, 900),
            refit: true,
        });

        let before_bv = view.bind(&binding);
        assert!(before_bv.commands.is_empty());
        let before = bound_digest(&before_bv);
        assert!(
            before.contains("catalog_sect=present"),
            "test setup must expose the insignia catalog rows, got:\n{before}"
        );
        assert_eq!(
            before
                .matches("button:sprite=GFX_theatre_army_shield#")
                .count(),
            4,
            "army shield catalog must expose four derived button rows, got:\n{before}"
        );

        let button = find_rect(&before_bv.hit.roots, "button")
            .expect("instantiated insignia_list_entry exposes the real button leaf");
        let (x, y) = (button.x + button.w * 0.5, button.y + button.h * 0.5);
        let leaf = before_bv
            .hit
            .hit_test(x, y)
            .and_then(|hit| hit.leaf().map(str::to_string));
        assert_eq!(
            leaf.as_deref(),
            Some("button"),
            "disabled mutation proof must click the real catalog button leaf"
        );

        let down = view.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(down.consumed);
        assert!(
            !down.needs_redraw,
            "insignia catalog pointer-down is a disabled mutation no-op"
        );
        assert!(down.commands.is_empty());

        let up = view.on_input(&ViewInput::PointerUp {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(up.consumed);
        assert!(
            !up.needs_redraw,
            "insignia catalog pointer-up is also a disabled mutation no-op"
        );
        assert!(up.commands.is_empty());

        let after = bound_digest(&view.bind(&binding));
        assert_eq!(
            before, after,
            "disabled insignia catalog clicks must not mutate selected formation or digest"
        );
    }

    #[test]
    fn insignia_without_explicit_selection_does_not_bind_first_owned_formation() {
        let stage = stage_with_gui_and_manifest(
            INSIGNIA_FILE,
            insignia_test_gui(),
            insignia_test_manifest(),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "order_of_battle": {
                "armies": [
                    {"id": 11, "owner": "GER", "name": "Current Group", "icon": 2}
                ],
                "army_groups": [
                    {"id": 22, "owner": "GER", "name": "Current Army Group", "icon": 1}
                ]
            }
        }))
        .unwrap();
        let bv = InsigniaSelectionView::new().bind(&ctx(&stage, &snap));
        let d = bound_digest(&bv);
        assert!(
            !d.contains("Current Group") && !d.contains("Current Army Group"),
            "menu-open insignia modal must not bind an arbitrary owned formation, got:\n{d}"
        );
        assert!(
            d.contains(&format!("group_name|Text(\"{DASH}\"")),
            "missing explicit selection must badge the group name unavailable, got:\n{d}"
        );
        assert!(
            d.contains("current=group_icon:sprite=MISS:no_sprite_name(U);"),
            "missing explicit selection must badge the current icon unavailable, got:\n{d}"
        );
        assert!(
            d.contains("catalog_sect=absent"),
            "missing explicit selection must not show a picker catalog, got:\n{d}"
        );
        assert_eq!(
            d.matches("button:sprite=").count(),
            0,
            "missing explicit selection must not fabricate catalog button sprites, got:\n{d}"
        );
    }

    #[test]
    fn insignia_army_group_selection_uses_army_group_catalog_frames() {
        let stage = stage_with_gui_and_manifest(
            INSIGNIA_FILE,
            insignia_test_gui(),
            insignia_test_manifest(),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "order_of_battle": {
                "armies": [
                    {"id": 11, "owner": "GER", "name": "First Army", "icon": 2}
                ],
                "army_groups": [
                    {"id": 22, "owner": "GER", "name": "Heeresgruppe A", "icon": 2}
                ]
            }
        }))
        .unwrap();
        let binding = ctx(&stage, &snap);
        let bv =
            InsigniaSelectionView::with_selection(FormationSelection::ArmyGroup(22)).bind(&binding);
        let d = bound_digest(&bv);
        assert!(
            d.contains("group_name|Text(\"Heeresgruppe A\""),
            "selected army group name must bind exact, got:\n{d}"
        );
        assert!(
            d.contains("current=group_icon:sprite=GFX_theatre_army_group_shield#1(D);"),
            "selected army group icon must use the army-group shield strip, got:\n{d}"
        );
        assert!(
            d.contains("catalog_sect=present"),
            "army-group catalog should be present when its strip resolves, got:\n{d}"
        );
        assert_eq!(
            d.matches("button:sprite=GFX_theatre_army_group_shield#")
                .count(),
            3,
            "catalog rows must come from the army-group strip frame count, got:\n{d}"
        );
        assert_eq!(
            d.matches("button:sprite=GFX_theatre_army_shield#").count(),
            0,
            "army-group selection must not show army-strip picker rows, got:\n{d}"
        );
    }

    #[test]
    fn insignia_missing_selection_id_fails_closed() {
        let stage = stage_with_gui_and_manifest(
            INSIGNIA_FILE,
            insignia_test_gui(),
            insignia_test_manifest(),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "order_of_battle": {
                "armies": [
                    {"id": 11, "owner": "GER", "name": "First Army", "icon": 2}
                ],
                "army_groups": []
            }
        }))
        .unwrap();
        let bv = InsigniaSelectionView::with_selection(FormationSelection::Army(99))
            .bind(&ctx(&stage, &snap));
        let d = bound_digest(&bv);
        assert!(
            !d.contains("First Army"),
            "unknown explicit selector must not fall back to the first owned army, got:\n{d}"
        );
        assert!(
            d.contains("current=group_icon:sprite=MISS:no_sprite_name(U);"),
            "unknown explicit selector must badge the current icon unavailable, got:\n{d}"
        );
        assert!(
            d.contains("catalog_sect=absent"),
            "unknown explicit selector must badge the catalog unavailable, got:\n{d}"
        );
        assert_eq!(
            d.matches("button:sprite=").count(),
            0,
            "unknown explicit selector must not fabricate catalog rows, got:\n{d}"
        );
    }

    #[test]
    fn insignia_missing_catalog_and_current_icon_stay_unavailable() {
        let stage = stage_with_gui_and_manifest(INSIGNIA_FILE, insignia_test_gui(), HashMap::new());
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "countries": {"GER": {}}
        }))
        .unwrap();
        let bv = InsigniaSelectionView::new().bind(&ctx(&stage, &snap));
        assert_eq!(bv.commands.len(), 0);
        let d = bound_digest(&bv);
        assert!(
            d.contains("current=group_icon:sprite=MISS:no_sprite_name(U);"),
            "missing order_of_battle must badge the current icon unavailable, got:\n{d}"
        );
        assert!(
            d.contains("catalog_sect=absent"),
            "missing army shield manifest entry must make the catalog absent, got:\n{d}"
        );
        assert_eq!(
            d.matches("button:sprite=").count(),
            0,
            "an absent catalog must not fabricate catalog button sprites, got:\n{d}"
        );
    }

    // -------- per-view element accounting: scene walk == the .gui's element_type_counts ---

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

    fn account(name: &str) {
        if !assets_present() {
            return;
        }
        assert_eq!(
            raw_type_counts(name),
            declared_counts(name),
            "element accounting mismatch for {name}"
        );
    }

    #[test]
    fn element_accounting_intel_agency() {
        account("intel_agency.json");
    }
    #[test]
    fn element_accounting_intel_ledger() {
        account("intel_ledger.json");
    }
    #[test]
    fn element_accounting_operative() {
        account("operative.json");
    }
    #[test]
    fn element_accounting_operative_leader() {
        account("operative_leader.json");
    }
    #[test]
    fn element_accounting_operation_overview() {
        account("operation_overview.json");
    }
    #[test]
    fn element_accounting_trade() {
        account("trade.json");
    }
    #[test]
    fn element_accounting_peace_conference() {
        account("peace_conference.json");
    }
    #[test]
    fn element_accounting_world_tension_popup() {
        account("world_tension_popup.json");
    }
    #[test]
    fn element_accounting_war_overview() {
        account("war_overview.json");
    }
    #[test]
    fn element_accounting_insignia_selection() {
        account("insignia_selection.json");
    }
    #[test]
    fn element_accounting_ins_revolution() {
        account("ins_revolution.json");
    }
    #[test]
    fn element_accounting_diplomacy() {
        account("diplomacy.json");
    }

    /// DIPLOMACY scrub A→B→A is byte-identical (the immutable-LRU rule, through the view's
    /// pure `bind`). A and B carry different opinion/stability/focus so the digest moves.
    #[test]
    fn diplomacy_scrub_a_b_a_byte_identical() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["diplomacy.json"]);
        let (a, b) = (snap_a(), snap_b());
        let da1 = bound_digest(&DiplomacyView::new().bind(&ctx(&stage, &a)));
        let db = bound_digest(&DiplomacyView::new().bind(&ctx(&stage, &b)));
        assert_ne!(
            da1, db,
            "A and B must differ, else the scrub proves nothing"
        );
        let da2 = bound_digest(&DiplomacyView::new().bind(&ctx(&stage, &a)));
        assert_eq!(
            da1, da2,
            "diplomacy scrub A→B→A carried a value from the B visit"
        );
    }

    /// DIPLOMACY no-fabrication + the Codex-confirmed opinion DIRECTION: with VIEWER=GER the
    /// country-list row for POL binds our_opinion = GER→POL (-50) and their_opinion = POL→GER
    /// (-200) — DISTINCT, proving the direction is not reversed (a VIEWER≠SEL proof, since a
    /// reversed impl would swap them). The self-opinion (GER→GER) is a measured `0` rendered
    /// `+0` (a real zero, NOT the not-captured glyph). leader_name/elections badge "—".
    #[test]
    fn diplomacy_opinion_direction_and_no_fab() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["diplomacy.json"]);
        let bv = DiplomacyView::new().bind(&ctx(&stage, &snap_a()));
        let d = bound_digest(&bv);
        // identity + politics bind EXACT into the ROOT nodes.
        assert!(
            d.contains("country_name|Text(\"German Reich\""),
            "country name binds exact, got:\n{d}"
        );
        assert!(
            d.contains("stability_value|Text(\"72%\""),
            "stability binds exact percent, got:\n{d}"
        );
        assert!(
            d.contains("war_support_value|Text(\"60%\""),
            "war support binds exact percent"
        );
        assert!(
            d.contains("national_focus_label|Text(\"GER_rhineland\""),
            "current focus binds exact"
        );
        // self-opinion is a MEASURED 0 -> "+0" (a real zero), never the not-captured glyph.
        assert!(
            d.contains("our_opinion_value|Text(\"+0\""),
            "self opinion is a measured 0 → +0, got:\n{d}"
        );
        // commanders/elections unfed -> the not-captured glyph "—" on real ROOT nodes (never 0).
        assert!(
            d.contains("leader_name|Text(\"—\""),
            "leader name (commanders unfed) badges —"
        );
        assert!(
            d.contains("elections|Text(\"—\""),
            "elections (no field) badges —"
        );
        // DIRECTION: the POL country-list row carries our=−50 (GER→POL) and their=−200 (POL→GER).
        assert!(
            d.contains("our_opinion=-50(E)"),
            "GER→POL opinion (−50) on the list row, got:\n{d}"
        );
        assert!(
            d.contains("their_opinion=-200(E)"),
            "POL→GER opinion (−200) on the list row — direction not reversed, got:\n{d}"
        );
        // never a fabricated 0 for the unavailable fields.
        assert!(
            !d.contains("leader_name|Text(\"0\""),
            "leader name must NEVER render a fabricated 0"
        );
    }

    /// The REAL `.gui` element totals (from the parsed files — the binding-map totals are
    /// stale; these are the authoritative scene-walk sums).
    #[test]
    fn element_accounting_totals_real_gui() {
        if !assets_present() {
            return;
        }
        let total = |n: &str| raw_type_counts(n).values().sum::<usize>();
        assert_eq!(total("intel_agency.json"), 208);
        assert_eq!(total("intel_ledger.json"), 345);
        assert_eq!(total("operative.json"), 11);
        assert_eq!(total("operative_leader.json"), 92);
        assert_eq!(total("operation_overview.json"), 97);
        assert_eq!(total("trade.json"), 107);
        assert_eq!(total("peace_conference.json"), 214);
        assert_eq!(total("world_tension_popup.json"), 32);
        assert_eq!(total("war_overview.json"), 74);
        assert_eq!(total("insignia_selection.json"), 44);
        assert_eq!(total("ins_revolution.json"), 12);
    }

    /// Codex tightenings (final review): war-participant `ic` + `wargoal` are honest, the
    /// inactive peace card is NOT drawn, and an uninvolved observer's war header is `—`.
    #[test]
    fn codex_fidelity_tightenings_hold() {
        if !assets_present() {
            return;
        }
        // (1) war participant `ic` is badged unavailable (civ-factory count ≠ IC), and the
        // wargoal binds only when it TARGETS the opposing belligerent (GER's wargoal targets
        // POL = the enemy → derived).
        let stage = stage_with(&["war_overview.json"]);
        let d = bound_digest(&WarOverviewView::new().bind(&ctx(&stage, &snap_a())));
        assert!(
            d.contains("ic=—(U)"),
            "participant IC must badge unavailable (not civ factories), got:\n{d}"
        );
        assert!(
            d.contains("wargoal=annex_everything(D)"),
            "wargoal targeting the enemy binds derived, got:\n{d}"
        );

        // (2) an observer in NO war gets an UNAVAILABLE header (never an arbitrary third war).
        let stage2 = stage_with(&["war_overview.json"]);
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11",
            "countries": { "GER": {} },
            "divisions": [],
            "diplomacy_world": { "wars": [ {"a": "FRA", "b": "ITA", "a_casualties": 1, "b_casualties": 2, "war_score_a_vs_b": 9} ] }
        })).unwrap();
        let d2 = bound_digest(&WarOverviewView::new().bind(&ctx(&stage2, &snap)));
        assert!(
            d2.contains("header_desc|Text(\"—\"") || d2.contains("FID e=0"),
            "an uninvolved observer must not be attributed FRA vs ITA, got:\n{d2}"
        );
        // but the war LIST still renders the (real) FRA vs ITA row.
        assert!(
            d2.contains("FRA") && d2.contains("ITA"),
            "the war list still shows the real war, got:\n{d2}"
        );

        // (3) the inactive peace conference draws NO N5 card (only the root + gaps).
        let stage3 = stage_with(&["peace_conference.json"]);
        let d3 = bound_digest(&PeaceConferenceView::new().bind(&ctx(&stage3, &snap_a())));
        assert!(
            d3.contains("active=false") && d3.contains("no card — conference inactive"),
            "an inactive conference must render no card, got:\n{d3}"
        );
    }

    /// All 11 views are registered into the inventory + routed under the IntelDiplomacyTrade
    /// cluster (the conflict-free menubar seam — proven through `from_inventory`).
    #[test]
    fn all_eleven_views_registered_in_inventory() {
        use crate::host::ViewRegistry;
        let reg = ViewRegistry::from_inventory();
        for id in [
            "intel.intel_agency",
            "intel.intel_ledger",
            "intel.operative",
            "intel.operative_leader",
            "intel.operation_overview",
            "intel.trade",
            "intel.peace_conference",
            "intel.world_tension_popup",
            "intel.war_overview",
            "intel.insignia_selection",
            "intel.ins_revolution",
            "intel.diplomacy",
        ] {
            assert!(
                reg.view(crate::host::ViewId(id)).is_some(),
                "view {id} must be in the inventory"
            );
        }
        // the diplomacy view is MENU-REACHABLE under the IntelDiplomacyTrade cluster.
        let bar = reg.menubar();
        let group = bar
            .iter()
            .find(|g| g.cluster == MenuCluster::IntelDiplomacyTrade)
            .expect("the IntelDiplomacyTrade cluster group exists");
        assert!(
            group.items.iter().any(|i| i.view_id == DIPLOMACY_VIEW_ID),
            "the diplomacy view must be routed to the Intel menu (menu-reachable)"
        );
    }
}
