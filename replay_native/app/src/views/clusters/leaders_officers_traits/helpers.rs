//! Shared no-fabrication plumbing for the leaders/officers/traits cluster (read via
//! [`Captured`], render via [`DisplayAtom`] — never a fabricated 0). Mirrors the worked
//! `politics_government.rs` helpers, kept cluster-local so NO shared file is edited.

use gui_runtime::draw::{DrawStats, TextureCache};
use gui_runtime::fonts::FontCache;
use gui_runtime::gridbox::{self, GridSpec};
use gui_runtime::scene::{ElementKind, Node};
use gui_runtime::{build_scene, layout, GuiFile, Loc, ManifestContent, Rect};

use bind::Fidelity;

use crate::host::context::{BindContext, Captured, DisplayAtom, FidelityReport, UnavailableReason};
use crate::host::draw::FrameDraw;

/// The not-captured glyph (the only thing a blocked/absent/null field renders).
pub const DASH: &str = "—";

/// The gridbox inter-row gutter (px).
pub const GUTTER: f32 = 14.0;

/// Read a JSON value out of an array ELEMENT as a [`Captured`] (preserving the
/// Absent/Null/Measured trichotomy at the leaf), so a per-row field is no-fabrication too.
pub fn elem_field<'a>(elem: &'a serde_json::Value, key: &str) -> Captured<&'a serde_json::Value> {
    crate::host::context::classify_value(elem.get(key))
}

/// Format a measured number as a truncated integer string.
pub fn fmt_int(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) => format!("{}", f.trunc() as i64),
        None => v
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| DASH.to_string()),
    }
}

/// Format a measured value as its raw string token (a name, role, TAG, idea token).
pub fn fmt_token(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => DASH.to_string(),
    }
}

/// Find a laid-out node's rect by name (depth-first), non-degenerate only.
pub fn find_rect(roots: &[Node], name: &str) -> Option<Rect> {
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
pub fn find_template(file: &GuiFile, name: &str) -> Option<Node> {
    let raw = file
        .windows
        .iter()
        .find(|w| w.name.as_deref() == Some(name))?;
    let manifest = assets::SpriteManifest {
        sprites: std::collections::HashMap::new(),
    };
    Some(Node::from_raw(raw, &manifest, &Loc::new()))
}

/// An empty `.gui` (the only thing returned when a file is genuinely unreadable — never a
/// fabricated layout).
pub fn empty_gui() -> GuiFile {
    GuiFile::parse(r#"{"view":"empty","windows":[]}"#).expect("empty gui parses")
}

/// Resolve a view's REAL parsed `.gui` file. Prefers the pre-loaded [`Stage::gui_files`]
/// entry (the conflict-free `.gui`-set seam); falls back to a LENIENT re-parse from
/// `Stage::assets_dir/gui/<file>` (coercing the two string-where-`f32` shapes the strict
/// typed parser rejects) so the view binds the REAL game layout, not synthetic chrome.
/// Returns [`empty_gui`] only if genuinely unparseable even after sanitising.
pub fn load_gui(ctx: &BindContext, file: &str) -> GuiFile {
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

/// Repair the two string-where-`f32`-expected shapes the strict `GuiFile` typed parser
/// rejects: a `position` object's `x`/`y` STRING → its numeric prefix; a `text` object's
/// `maxWidth`/`maxHeight` STRING → DROPPED (the faithful recovery — no fabricated number).
pub fn sanitize_positions(v: &mut serde_json::Value) {
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

/// Parse the leading signed-NUMBER prefix of a string + truncate to an integer pixel
/// (`"1s"` → 1, `"-2px"` → -2, `"-0.5px"` → 0, `"abc"` → 0). Used ONLY to recover a
/// malformed `.gui` `position` scalar (a layout coordinate) — NOT a snapshot value.
pub fn leading_number(s: &str) -> i64 {
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
pub struct Cell {
    pub element: String,
    pub atom: DisplayAtom,
}

#[derive(Clone)]
pub struct SpriteCell {
    pub element: String,
    pub asset: assets::SpriteAsset,
    pub fidelity: Fidelity,
}

/// One instantiated data row: the element→atom cells. Custom-drawn through the template;
/// recorded in the digest so a scrub is exact. `gaps` holds LOGICAL unavailable fields with
/// NO backing `.gui` node — recorded for honesty, never bound.
#[derive(Clone)]
pub struct DataRow {
    pub cells: Vec<Cell>,
    pub sprites: Vec<SpriteCell>,
    pub gaps: Vec<String>,
}

impl DataRow {
    pub fn new() -> DataRow {
        DataRow {
            cells: Vec::new(),
            sprites: Vec::new(),
            gaps: Vec::new(),
        }
    }
    /// Bind `element` to `cap` formatted by `fmt` at `fidelity`. Absent/Null render "—".
    pub fn bind(
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
    /// Push an already-built atom for `element` (when the value was joined/derived).
    pub fn push_atom(&mut self, report: &mut FidelityReport, element: &str, atom: DisplayAtom) {
        report.record(&atom);
        self.cells.push(Cell {
            element: element.to_string(),
            atom,
        });
    }
    /// Bind `element` to an explicit `unavailable` badge ("—") — runtime-computed /
    /// live-resisted / DEF-data we never fabricate. `element` MUST name a real `.gui` node.
    pub fn badge_unavailable(&mut self, report: &mut FidelityReport, element: &str) {
        let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        report.record(&atom);
        self.cells.push(Cell {
            element: element.to_string(),
            atom,
        });
    }

    pub fn bind_sprite(
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
}

impl Default for DataRow {
    fn default() -> Self {
        DataRow::new()
    }
}

/// A digest line for a set of data rows (stable, comparable across a scrub).
pub fn row_digest(rows: &[DataRow]) -> String {
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

/// Why a row section has no rows — so "no data" (live channel unfed) is never conflated
/// with "empty list" (measured, genuinely zero rows). (Codex no-fab #3.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionState {
    Present,
    Absent,
    Null,
    Degraded,
}

impl SectionState {
    pub fn record(self, report: &mut FidelityReport) {
        match self {
            SectionState::Present => report.exact += 1,
            _ => report.unavailable += 1,
        }
    }
    pub fn tag(self) -> &'static str {
        match self {
            SectionState::Present => "present",
            SectionState::Absent => "absent",
            SectionState::Null => "null",
            SectionState::Degraded => "degraded",
        }
    }
}

/// Read an array field under the observed country as a slice, preserving the trichotomy.
pub fn country_array<'a>(
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

/// Read a nested array out of a TOP-LEVEL domain `Captured` value (e.g.
/// `order_of_battle.fleets` / `extra.aces`), preserving the trichotomy. The `path` is a
/// `/`-joined JSON pointer tail under the measured domain object.
pub fn domain_array<'a>(
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

pub fn portrait_asset(ctx: &BindContext, record: &serde_json::Value) -> assets::SpriteAsset {
    assets::resolve_portrait_sprite(&ctx.stage.manifest, record)
}

pub fn commander_portrait_for_ref(
    ctx: &BindContext,
    tag: &str,
    leader_ref: Option<&serde_json::Value>,
    sources: &[&str],
) -> assets::SpriteAsset {
    match find_commander_by_leader_ref(ctx, tag, leader_ref, sources) {
        Some(commander) => portrait_asset(ctx, commander),
        None => assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName),
    }
}

pub fn land_oob_portrait_asset(ctx: &BindContext, tag: &str) -> assets::SpriteAsset {
    let oob = ctx.order_of_battle();
    for pointer in ["/armies", "/army_groups"] {
        let (items, _) = domain_array(&oob, pointer);
        if let Some(items) = items {
            for item in items {
                match elem_field(item, "owner") {
                    Captured::Measured(owner) if owner.as_str() == Some(tag) => {}
                    _ => continue,
                }
                let direct = portrait_asset(ctx, item);
                if !matches!(
                    direct,
                    assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName)
                ) {
                    return direct;
                }
                let sources = if pointer == "/army_groups" {
                    &["commanders.field_marshals"][..]
                } else {
                    &["commanders.generals", "commanders.field_marshals"][..]
                };
                let joined = commander_portrait_for_ref(ctx, tag, item.get("leader"), sources);
                if !matches!(
                    joined,
                    assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName)
                ) {
                    return joined;
                }
            }
        }
    }
    assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName)
}

pub fn find_commander_by_leader_ref<'a>(
    ctx: &'a BindContext,
    tag: &str,
    leader_ref: Option<&serde_json::Value>,
    sources: &[&str],
) -> Option<&'a serde_json::Value> {
    let keys = leader_ref_keys(leader_ref?);
    if keys.is_empty() {
        return None;
    }
    for source in sources {
        if let (Some(commanders), SectionState::Present) = country_array(ctx, tag, source) {
            for commander in commanders {
                if commander_matches_keys(commander, &keys) {
                    return Some(commander);
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum LeaderRefKey {
    Token(String),
    Typed { id: String, kind: String },
}

fn commander_matches_keys(commander: &serde_json::Value, keys: &[LeaderRefKey]) -> bool {
    if keys_overlap(&leader_ref_keys(commander), keys) {
        return true;
    }
    for field in [
        "unit_leader_id",
        "id",
        "leader",
        "token",
        "char_id",
        "character",
    ] {
        if let Some(v) = commander.get(field) {
            if keys_overlap(&leader_ref_keys(v), keys) {
                return true;
            }
        }
    }
    false
}

fn keys_overlap(a: &[LeaderRefKey], b: &[LeaderRefKey]) -> bool {
    a.iter().any(|candidate| b.iter().any(|k| k == candidate))
}

fn leader_ref_keys(v: &serde_json::Value) -> Vec<LeaderRefKey> {
    let mut keys = Vec::new();
    match v {
        serde_json::Value::String(s) => push_ref_string(&mut keys, s),
        serde_json::Value::Number(n) => keys.push(LeaderRefKey::Token(n.to_string())),
        serde_json::Value::Object(map) => {
            let typed_id = map
                .get("id")
                .or_else(|| map.get("unit_leader_id"))
                .or_else(|| map.get("leader"))
                .filter(|v| leader_ref_scalars(v).len() == 1);
            if let (Some(id), Some(kind)) = (typed_id, map.get("type")) {
                for id_key in leader_ref_scalars(id) {
                    for kind_key in leader_ref_scalars(kind) {
                        keys.push(LeaderRefKey::Typed {
                            id: id_key.clone(),
                            kind: kind_key,
                        });
                    }
                }
            }
            for key in ["token", "char_id", "character"] {
                if let Some(child) = map.get(key) {
                    keys.extend(leader_ref_keys(child));
                }
            }
            if typed_id.is_none() {
                for key in ["id", "unit_leader_id", "leader"] {
                    if let Some(child) = map.get(key) {
                        keys.extend(leader_ref_keys(child));
                    }
                }
            } else if let Some(child) = map.get("leader").filter(|v| v.is_string()) {
                keys.extend(leader_ref_keys(child));
            }
        }
        _ => {}
    }
    keys.sort();
    keys.dedup();
    keys
}

fn leader_ref_scalars(v: &serde_json::Value) -> Vec<String> {
    let mut values = Vec::new();
    match v {
        serde_json::Value::String(s) => {
            let s = s.trim();
            if !s.is_empty() {
                values.push(s.to_string());
            }
        }
        serde_json::Value::Number(n) => values.push(n.to_string()),
        serde_json::Value::Object(map) => {
            for key in ["id", "unit_leader_id", "leader", "type"] {
                if let Some(child) = map.get(key) {
                    values.extend(leader_ref_scalars(child));
                }
            }
        }
        _ => {}
    }
    values.sort();
    values.dedup();
    values
}

fn push_ref_string(keys: &mut Vec<LeaderRefKey>, raw: &str) {
    let s = raw.trim();
    if s.is_empty() {
        return;
    }
    keys.push(LeaderRefKey::Token(s.to_string()));
    if let Some((id, kind)) = s.split_once(':') {
        let id = id.trim();
        let kind = kind.trim();
        if !id.is_empty() && !kind.is_empty() {
            keys.push(LeaderRefKey::Typed {
                id: id.to_string(),
                kind: kind.to_string(),
            });
        }
    }
}

/// Push a single summary cell bound from one country path at a chosen fidelity.
pub fn push_country(
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

/// Push an explicit `unavailable` summary cell (runtime-computed / DEF / FOLD-IN). "—".
pub fn push_unavailable(report: &mut FidelityReport, summary: &mut Vec<Cell>, element: &str) {
    let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

pub fn push_sprite(
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

/// Set the bound summary atoms into the named ROOT scene tree + lay it out. Returns the
/// laid-out root windows for the named ROOT (only).
pub fn build_root(
    ctx: &BindContext,
    file: &GuiFile,
    root_name: &str,
    summary: &[Cell],
) -> Vec<Node> {
    build_root_with_sprites(ctx, file, root_name, summary, &[])
}

pub fn build_root_with_sprites(
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

/// A stub gridbox node carrying the viewport rect (the gridbox spec needs a `gridBoxType`).
pub fn stub_grid(name: &str, rect: Rect) -> Node {
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

/// The shared per-row custom draw: instantiate the template into the viewport gridbox and
/// stamp each row's cell atoms into the named children, then draw the rows + scrollbar.
/// (Mirrors `politics_government::render_rows`.)
pub fn render_rows(
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
        for sprite in &rows[i].sprites {
            apply_sprite_cell(row, sprite);
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

pub fn record_sprite_asset(
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

pub fn sprite_digest(sprite: &SpriteCell) -> String {
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

pub fn sprites_digest(sprites: &[SpriteCell]) -> String {
    let mut s = String::new();
    for sprite in sprites {
        s.push_str(&sprite_digest(sprite));
    }
    s
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

pub fn apply_sprite_cell(root: &mut Node, sprite: &SpriteCell) -> bool {
    match &sprite.asset {
        assets::SpriteAsset::Resolved(resolved) => {
            set_element_sprite(root, &sprite.element, &resolved.name, resolved.frame)
        }
        assets::SpriteAsset::Unavailable(_) => false,
    }
}

pub fn set_element_sprite(node: &mut Node, name: &str, sprite: &str, frame: u32) -> bool {
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

/// Mint a per-frame 1x1 white texture (the substrate every widget tints).
pub fn mint_white(fd: &FrameDraw) -> render::Texture {
    widgets::common::white_texture(fd.renderer)
}

/// Geometry-only `widgets::DrawAssets` over an owned white texture (no font → the widget
/// layer is always visible + never fabricates a label).
pub fn widget_assets(white: &render::Texture) -> widgets::common::DrawAssets<'_> {
    widgets::common::DrawAssets {
        white,
        font: None,
        font_pages: &[],
    }
}
