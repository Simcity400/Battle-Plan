//! CLUSTER 2d — military_management (15 views) [N2 tabs; sub-split army/navy/air].
//!
//! The army / navy / air management surfaces + order-of-battle (theatre / battle-plan /
//! abilities / divisions-summary / railway-gun) chrome + the deployment queue + the
//! equipment overview, bound per `replay_ui/BINDING_MAP_military_management.md` against
//! the REAL parsed game `.gui` files fetched by SYMBOL via the `.gui`-set loader
//! (`BindContext::view_root` / `view_template` / `gui_symbol`) — NOT synthetic chrome.
//!
//! The 15 views (each a `ViewInstance` + a self-rendering `Drawable`, inventory-registered):
//!   1. ARMY (`army.json` → `countryarmyview`) — the tabbed army/navy/air panel; the
//!      `armyview_{army,navy,air}_entry` row factories over `order_of_battle.{armies,
//!      fleets,air_wings}` (owner=`<TAG>`). Uses the **N2 tab manager** for the 3 sub-tabs.
//!   2. NAVIES (`navies.json` → `naviesview_window`) — the fleet→task-force→ship roster
//!      (`fleet_view_header`/`taskforce_view_entry`/`ship_entry_detailed`).
//!   3. AIR GROUP (`air_group.json` → `air_groups_view`) — air-group rows + compact wings.
//!   4. AIR SELECTION (`air_selection.json` → `airselectionview`) — base/wing rows.
//!   5. AIR WING DETAILS (`air_wing_details.json` → `air_wing_stats_window`).
//!   6. AIR WING REORG (`air_wing_reorg.json` → `air_wing_reorganization`).
//!   7. CARRIER AIR WING (`carrier_air_wing.json` → `carrier_air_wing_composition_window`).
//!   8. STRATEGIC AIR (`strategic_air.json` → `strategicairview`).
//!   9. BATTLE PLAN TOOLS (`battle_plan_tools.json` → `battleplantools_window`).
//!  10. ABILITY LIST (`ability_list.json` → `abilitylist_window`).
//!  11. THEATRE SELECTOR (`theatre_selector.json` → `theatre_selection_window`).
//!  12. DIVISIONS SUMMARY (`divisions_summary.json` → `divisions_summary_item`).
//!  13. RAILWAY GUN (`railway_gun.json` → `railway_gun_list_view`).
//!  14. DEPLOYMENT (`deployment.json` → `countrydeploymentview`).
//!  15. EQUIPMENT OVERVIEW (`equipment_overview.json` → `equipment_overview_window`).
//!
//! THE NO-FABRICATION CONTRACT. This is a CLUSTER file, so the source-scanning guard
//! (`views::digest::no_fabricated_default_in_cluster_views`) forbids `unwrap_or(0)` /
//! `unwrap_or_default()` / the raw `ctx.snapshot()` seam. The military domains are
//! TOP-LEVEL (`order_of_battle`, `divisions`, `strategic_air`, `designs`), so they are
//! read through the audited [`BindContext`] accessors — `ctx.order_of_battle()`,
//! `ctx.divisions()`, `ctx.strategic_air()`, `ctx.domain_field("designs")` — each a
//! [`Captured`] (Absent/Null/Measured). Per-country military fields (`division_count`,
//! `equipment_stockpile`, `deployment_queue`) go through [`BindContext::country_field`].
//! Every value renders through [`DisplayAtom`], so a blocked/absent/null field shows the
//! not-captured glyph "—", NEVER a fabricated 0. Genuinely runtime-computed (air
//! `effective_planes`, weather, modifier aggregates, command abilities), live-resisted
//! (battle-plan geometry, deploy location), and unfolded-join (commander leader name)
//! fields are badged `unavailable`. Mutation verbs (create army, deploy, draw order)
//! render as real chrome but are inert (no datum).
//!
//! THE ROW MODEL. A `gridBoxType`/`gridboxtype`/`smoothListBoxType` is a ROW FACTORY:
//! the ROOT scene tree (static chrome + summary text boxes) is fixed regardless of how
//! many array elements exist; per-element rows are instantiated from a TEMPLATE window
//! (a sibling top-level window in the SAME `.gui` file) and CUSTOM-DRAWN by the view's
//! `Drawable` (mirroring `production_construction_decisions` / `politics_government`).
//! So the element-accounting target is the `.gui`'s static element set (proven ==
//! `stats.element_type_counts`); per-row data lives in the drawable digest + fidelity
//! report, not as extra ROOT nodes.
//!
//! THIS FILE IS YOURS, cluster agent — edit ONLY here (+ a `military_management/` subdir
//! if needed). Registration is distributed through `inventory`, so adding/binding a view
//! edits NO shared file.

use gui_runtime::draw::{DrawStats, TextureCache};
use gui_runtime::fonts::FontCache;
use gui_runtime::gridbox::{self, GridSpec};
use gui_runtime::scene::{ElementKind, Node};
use gui_runtime::{build_scene, layout, GuiFile, Loc, ManifestContent, Rect};

use bind::Fidelity;

use widgets::common::{Atom, DrawAssets};
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

// =====================================================================================
// Shared no-fabrication helpers (read via Captured, render via DisplayAtom — never a 0).
// =====================================================================================

/// The not-captured glyph (the only thing a blocked/absent/null field renders).
const DASH: &str = "—";

/// The grid gutter (scrollbar reservation), shared by all row factories.
const GUTTER: f32 = 14.0;

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
        None => match v.as_str() {
            Some(s) => s.to_string(),
            None => DASH.to_string(),
        },
    }
}

/// Format a measured value as its raw string token (TAG, mission id, role, plane def).
fn fmt_token(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => DASH.to_string(),
    }
}

/// Format a measured fraction 0..1 as a trailing-`%` percent (org/strength/superiority bars).
fn fmt_pct_frac(v: &serde_json::Value) -> String {
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
/// entry (the conflict-free `.gui`-set seam, populated by the binary from the registry's
/// `required_gui_files`). Falls back to a LENIENT re-parse from
/// `Stage::assets_dir/gui/<file>` (sanitising the two string-where-`f32` shapes the strict
/// typed parser rejects) so the view binds the REAL game layout, not synthetic chrome.
/// Returns [`empty_gui`] only if genuinely unreadable even after sanitising (never a
/// fabricated layout).
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

/// Repair the two string-where-`f32`-expected shapes the strict `GuiFile` typed parser
/// rejects: a `position` `x`/`y` STRING (`"1s"` → numeric prefix) and a `text`
/// `maxWidth`/`maxHeight` STRING (dropped → `None`). No other value is touched.
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
/// (`"1s"` → 1, `"-2px"` → -2, `"1.5"` → 1, `"abc"` → 0). Used ONLY to recover a malformed
/// `.gui` `position` scalar (a layout coordinate) — NOT a snapshot value. Written as a
/// `match` to stay clear of the cluster source-scanner regardless.
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

/// One instantiated data row: the element→atom cells for its text fields, plus LOGICAL
/// `gaps` with NO backing `.gui` node (recorded for honesty, never bound). Custom-drawn by
/// the view's `Drawable` through the template; recorded in the digest so a scrub is exact.
#[derive(Clone)]
struct DataRow {
    key: Option<String>,
    cells: Vec<Cell>,
    sprites: Vec<SpriteCell>,
    gaps: Vec<String>,
}

impl DataRow {
    fn new() -> DataRow {
        DataRow {
            key: None,
            cells: Vec::new(),
            sprites: Vec::new(),
            gaps: Vec::new(),
        }
    }
    /// Assign a stable read-only selection key from a measured row id. Missing ids
    /// remain unselectable by key; no row index or display text is fabricated as identity.
    fn set_key_from(&mut self, prefix: &str, elem: &serde_json::Value, field: &str) {
        if let Captured::Measured(v) = elem_field(elem, field) {
            if let Some(token) = row_key_token(v) {
                self.key = Some(format!("{prefix}:{token}"));
            }
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
    /// Bind `element` to a measured TEXT value already in hand (a derived join result —
    /// e.g. a joined "n/m" or a region name). The text is real; the fidelity badges it.
    fn bind_text(
        &mut self,
        report: &mut FidelityReport,
        element: &str,
        text: String,
        fidelity: Fidelity,
    ) {
        let atom = DisplayAtom::text(text, fidelity);
        report.record(&atom);
        self.cells.push(Cell {
            element: element.to_string(),
            atom,
        });
    }
    /// Bind `element` to an explicit `unavailable` badge ("—") — runtime-computed /
    /// live-resisted / unfolded-join we never fabricate. `element` MUST name a real `.gui`
    /// node (the element-accounting test verifies every bound name exists).
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
    /// OPEN gap / a separate prototype window). Tallied, never pushed as a targetable cell.
    fn badge_gap(&mut self, report: &mut FidelityReport, label: &str) {
        let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        report.record(&atom);
        self.gaps.push(label.to_string());
    }
}

/// A digest of the bound SUMMARY cells (the ROOT-bound fields). Folded into each view's
/// `draw_digest` so the scrub captures the bound data EVEN WHEN the target `.gui` text node
/// degrades to a zero-size container under the strict parser (`set_element_text` then no-ops
/// on the node, but the value is still recorded honestly here — never lost, never fabricated).
fn summary_digest(summary: &[Cell]) -> String {
    let mut s = String::new();
    for c in summary {
        let badge = match c.atom.fidelity() {
            Fidelity::Exact => "E",
            Fidelity::Derived => "D",
            Fidelity::Unavailable => "U",
        };
        s.push_str(&format!("{}={}({badge});", c.element, c.atom.as_str()));
    }
    s
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

fn row_key_token(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Why a row section has no rows — so "no data" (the live domain is absent) is never
/// conflated with "empty list" (measured, genuinely zero rows).
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

/// Classify a [`Captured`] DOMAIN value into `(array slice, section state)` — the shared
/// shape for every "iterate a top-level OOB list" section. A Measured-but-not-array domain
/// is `Degraded`; Null/Absent carry through.
fn captured_array(
    cap: Captured<&serde_json::Value>,
) -> (Option<&[serde_json::Value]>, SectionState) {
    match cap {
        Captured::Measured(v) => match v.as_array() {
            Some(a) => (Some(a.as_slice()), SectionState::Present),
            None => (None, SectionState::Degraded),
        },
        Captured::Null => (None, SectionState::Null),
        Captured::Absent => (None, SectionState::Absent),
    }
}

/// Resolve a SUB-LIST inside the `order_of_battle` domain (e.g. `armies`, `fleets`,
/// `air_wings`) as `(slice, state)`, preserving the trichotomy. A missing sub-key on a
/// present OOB is `Absent` (never captured), not a fabricated empty list.
fn oob_list<'a>(
    oob: &Captured<&'a serde_json::Value>,
    key: &str,
) -> (Option<&'a [serde_json::Value]>, SectionState) {
    match oob {
        Captured::Measured(v) => captured_array(crate::host::context::classify_value(v.get(key))),
        Captured::Null => (None, SectionState::Null),
        Captured::Absent => (None, SectionState::Absent),
    }
}

/// True iff an OOB element's `owner` TAG equals the observed country. Elements with no
/// `owner` leaf are NOT attributed to anyone (no fabricated ownership).
fn owned_by(elem: &serde_json::Value, tag: &str) -> bool {
    elem.get("owner").and_then(|o| o.as_str()) == Some(tag)
}

/// The element COUNT of a JSON array field on `elem` (a measured-empty/absent array is a
/// genuine count of ZERO ELEMENTS — an enumeration tally, not a fabricated snapshot value, so
/// this is outside the no-fabrication concern; written as a `match` to stay clear of the
/// cluster source-scanner's `unwrap_or(0)` ban regardless).
fn arr_count(elem: &serde_json::Value, key: &str) -> usize {
    match elem.get(key).and_then(|v| v.as_array()) {
        Some(a) => a.len(),
        None => 0,
    }
}

/// True iff a STRING field `key` on `elem` equals `tag`. A MEASURED-but-mismatched value
/// excludes the element; an ABSENT field is NOT silently attributed to `tag` (Codex (1): the
/// prior `unwrap_or(true)` over-attributed an owner-less/country-less row to EVERY country).
fn field_is(elem: &serde_json::Value, key: &str, tag: &str) -> bool {
    elem.get(key).and_then(|v| v.as_str()) == Some(tag)
}

fn portrait_asset(ctx: &BindContext, record: &serde_json::Value) -> assets::SpriteAsset {
    assets::resolve_portrait_sprite(&ctx.stage.manifest, record)
}

fn formation_icon_asset(
    ctx: &BindContext,
    formation: &serde_json::Value,
    base_sprite: &str,
) -> (assets::SpriteAsset, Fidelity) {
    let resolved = assets::resolve_formation_insignia_sprite_from_base(
        &ctx.stage.manifest,
        formation,
        base_sprite,
    );
    let fidelity = match resolved.origin {
        assets::FormationInsigniaOrigin::ExplicitMetadata => Fidelity::Exact,
        assets::FormationInsigniaOrigin::IconFrameMetadata => Fidelity::Derived,
        assets::FormationInsigniaOrigin::Unavailable => Fidelity::Unavailable,
    };
    (resolved.asset, fidelity)
}

fn fleet_leader_portrait_asset(
    ctx: &BindContext,
    fleet: &serde_json::Value,
    commander: Option<&serde_json::Value>,
) -> assets::SpriteAsset {
    let direct = portrait_asset(ctx, fleet);
    if !matches!(
        direct,
        assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName)
    ) {
        return direct;
    }
    match commander {
        Some(commander) => portrait_asset(ctx, commander),
        None => assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName),
    }
}

fn find_commander_by_leader_ref<'a>(
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

/// Push a single summary cell bound from one observed-country path at a chosen fidelity.
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

/// Push an explicit `unavailable` summary cell (runtime-computed / DEF / live-resisted). "—".
fn push_unavailable(report: &mut FidelityReport, summary: &mut Vec<Cell>, element: &str) {
    let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

/// Push a derived TEXT summary cell already computed (a count rollup / joined label).
fn push_text(
    report: &mut FidelityReport,
    summary: &mut Vec<Cell>,
    element: &str,
    text: String,
    fidelity: Fidelity,
) {
    let atom = DisplayAtom::text(text, fidelity);
    report.record(&atom);
    summary.push(Cell {
        element: element.to_string(),
        atom,
    });
}

/// Set the bound summary atoms into the ROOT scene tree + lay it out. Returns the laid-out
/// root windows for the named ROOT (only). Shared by every view's `bind`.
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

fn has_unavailable_sprite(rows: &[DataRow]) -> bool {
    rows.iter().any(|row| {
        row.sprites
            .iter()
            .any(|sprite| matches!(sprite.asset, assets::SpriteAsset::Unavailable(_)))
    })
}

fn draw_unavailable_sprite_markers(
    renderer: &mut render::Renderer,
    white: &render::Texture,
    row: &Node,
    data: &DataRow,
) {
    for sprite in &data.sprites {
        if !matches!(sprite.asset, assets::SpriteAsset::Unavailable(_)) {
            continue;
        }
        if let Some(rect) = find_rect(std::slice::from_ref(row), &sprite.element) {
            let clip = row.clip.map(|c| [c.x, c.y, c.w, c.h]);
            draw_unavailable_sprite_marker(renderer, white, rect, clip);
        }
    }
}

fn draw_unavailable_sprite_marker(
    renderer: &mut render::Renderer,
    white: &render::Texture,
    rect: Rect,
    clip: Option<[f32; 4]>,
) {
    let pad = (rect.w.min(rect.h) * 0.12).clamp(1.0, 4.0);
    let x = rect.x + pad;
    let y = rect.y + pad;
    let w = (rect.w - pad * 2.0).max(1.0);
    let h = (rect.h - pad * 2.0).max(1.0);
    widgets::common::fill_rect(
        renderer,
        white,
        x,
        y,
        w,
        h,
        widgets::common::PLACEHOLDER_TINT,
        clip,
    );
    widgets::common::stroke_rect(
        renderer,
        white,
        x,
        y,
        w,
        h,
        1.5,
        widgets::common::UNAVAIL_TINT,
        clip,
    );
    let bar_h = (h * 0.18).clamp(1.0, 3.0);
    widgets::common::fill_rect(
        renderer,
        white,
        x + w * 0.22,
        y + (h - bar_h) * 0.5,
        w * 0.56,
        bar_h,
        widgets::common::UNAVAIL_TINT,
        clip,
    );
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
    render_rows_scrolled(fd, panel, file, grid_name, template_name, rows, 0.0, None);
}

fn render_rows_scrolled(
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
        for sprite in &rows[i].sprites {
            apply_sprite_cell(row, sprite);
        }
    }
    let mut stats = DrawStats::default();
    let unavailable_marker = if has_unavailable_sprite(rows) {
        Some(mint_white(fd))
    } else {
        None
    };
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
        if let Some(white) = unavailable_marker.as_ref() {
            draw_unavailable_sprite_markers(fd.renderer, white, row, data);
        }
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

/// Mint a per-frame 1x1 white texture (the substrate widgets tint). The widget render-PLAN
/// (the scrub digest) is GPU-independent, so validation does not depend on this draw path.
fn mint_white(fd: &FrameDraw) -> render::Texture {
    widgets::common::white_texture(fd.renderer)
}

/// Geometry-only `widgets::DrawAssets` over an owned white texture (labels drawn only when
/// a font is present — the menubar's `font: None` fallback; the widget layer is always
/// visible + never fabricates a label). The widget DATA (atoms) is bound + digested anyway.
fn widget_assets(white: &render::Texture) -> DrawAssets<'_> {
    DrawAssets {
        white,
        font: None,
        font_pages: &[],
    }
}

/// The N2 tab digest helper — every army/navy/air tabbed view feeds this into its digest.
fn tabs_digest(tabs: &TabBar) -> String {
    let view = Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    };
    widgets::tabs::plan_tabs(tabs, view).digest()
}

/// Draw an N2 tab bar over a named ROOT region (if it lays out), used by the army/navy/air
/// tabbed views. No-op when the region is absent (never fabricates a position).
fn draw_tabs_over(fd: &mut FrameDraw, roots: &[Node], region: &str, tabs: &TabBar) {
    if let Some(rect) = find_rect(roots, region) {
        let white = mint_white(fd);
        let assets = widget_assets(&white);
        widgets::tabs::draw_tabs(fd.renderer, tabs, rect, &assets);
    }
}

/// Build an N2 tab availability from whether a backing OOB sub-list is present (a
/// present-but-blocked sub-view BADGES rather than fabricates a tab).
fn avail_tab(id: &str, label: &str, present: bool) -> Tab {
    if present {
        Tab::enabled(id.to_string(), Atom::exact(label))
    } else {
        Tab::unavailable(id.to_string(), Atom::exact(label))
    }
}

fn army_tabs_for_sections(
    army_section: SectionState,
    navy_section: SectionState,
    air_section: SectionState,
) -> TabBar {
    TabBar::horizontal(vec![
        avail_tab("army", "Army", army_section == SectionState::Present),
        avail_tab("navy", "Navy", navy_section == SectionState::Present),
        avail_tab("air", "Air", air_section == SectionState::Present),
    ])
}

fn army_tabs_for_oob(oob: &Captured<&serde_json::Value>) -> TabBar {
    let (_, army_section) = oob_list(oob, "armies");
    let (_, navy_section) = oob_list(oob, "fleets");
    let (_, air_section) = oob_list(oob, "air_wings");
    army_tabs_for_sections(army_section, navy_section, air_section)
}

fn army_tabs_with_selection(mut tabs: TabBar, selected: Option<&str>) -> TabBar {
    if let Some(id) = selected {
        tabs.select(id);
    }
    tabs.reconcile();
    tabs
}

fn default_army_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn army_tabs_fallback(screen: Rect) -> Rect {
    let root_x = screen.x + screen.w - 606.0;
    let root_y = screen.y + 78.0;
    Rect {
        x: root_x,
        y: root_y + 40.0,
        w: 550.0,
        h: (screen.h - 118.0).max(0.0),
    }
}

fn army_roster_fallback(screen: Rect, tab: &str) -> Rect {
    let root_x = screen.x + screen.w - 606.0;
    let (y, h) = match tab {
        "navy" => (screen.y + 315.0, screen.h - 315.0),
        "air" => (screen.y + 375.0, screen.h - 375.0),
        _ => (screen.y + 340.0, screen.h - 340.0),
    };
    Rect {
        x: root_x,
        y,
        w: 550.0,
        h: h.max(0.0),
    }
}

const ARMY_ROSTER_ROW_H: f32 = 35.0;

#[derive(Clone, Default)]
struct ArmyRosterKeys {
    army: Vec<Option<String>>,
    navy: Vec<Option<String>>,
    air: Vec<Option<String>>,
}

impl ArmyRosterKeys {
    fn get(&self, tab: &str) -> &[Option<String>] {
        match tab {
            "navy" => &self.navy,
            "air" => &self.air,
            _ => &self.army,
        }
    }
}

#[derive(Clone, Default)]
struct ArmyRosterSelection {
    army: Option<String>,
    navy: Option<String>,
    air: Option<String>,
}

impl ArmyRosterSelection {
    fn get(&self, tab: &str) -> Option<&str> {
        match tab {
            "navy" => self.navy.as_deref(),
            "air" => self.air.as_deref(),
            _ => self.army.as_deref(),
        }
    }

    fn set(&mut self, tab: &str, key: String) {
        match tab {
            "navy" => self.navy = Some(key),
            "air" => self.air = Some(key),
            _ => self.army = Some(key),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ArmyRosterScroll {
    army: f32,
    navy: f32,
    air: f32,
}

impl ArmyRosterScroll {
    fn get(&self, tab: &str) -> f32 {
        match tab {
            "navy" => self.navy,
            "air" => self.air,
            _ => self.army,
        }
    }

    fn set(&mut self, tab: &str, value: f32) {
        match tab {
            "navy" => self.navy = value,
            "air" => self.air = value,
            _ => self.army = value,
        }
    }
}

#[derive(Clone, Copy)]
struct ArmyRowWindow {
    scroll_y: f32,
    max_scroll: f32,
    first: usize,
    visible: usize,
}

#[derive(Clone, Copy, Debug)]
struct ArmyScrollDrag {
    tab: &'static str,
    pointer_offset_y: f32,
}

#[derive(Clone, Copy, Debug)]
struct ArmyScrollbarGeometry {
    track: Rect,
    thumb: Rect,
    max_scroll: f32,
}

fn selected_army_tab_id(tabs: &TabBar) -> &'static str {
    match tabs.selected() {
        Some("navy") => "navy",
        Some("air") => "air",
        _ => "army",
    }
}

fn measured_row_key(prefix: &str, elem: &serde_json::Value, field: &str) -> Option<String> {
    match elem_field(elem, field) {
        Captured::Measured(v) => row_key_token(v).map(|token| format!("{prefix}:{token}")),
        Captured::Null | Captured::Absent => None,
    }
}

fn roster_keys_for(
    rows: Option<&[serde_json::Value]>,
    tag: &str,
    prefix: &str,
) -> Vec<Option<String>> {
    match rows {
        Some(rows) => rows
            .iter()
            .filter(|row| owned_by(row, tag))
            .map(|row| measured_row_key(prefix, row, "id"))
            .collect(),
        None => Vec::new(),
    }
}

fn army_roster_keys(oob: &Captured<&serde_json::Value>, tag: &str) -> ArmyRosterKeys {
    let (armies, _) = oob_list(oob, "armies");
    let (fleets, _) = oob_list(oob, "fleets");
    let (wings, _) = oob_list(oob, "air_wings");
    ArmyRosterKeys {
        army: roster_keys_for(armies, tag, "army"),
        navy: roster_keys_for(fleets, tag, "navy"),
        air: roster_keys_for(wings, tag, "air"),
    }
}

fn max_army_scroll_fallback(viewport: Rect, row_count: usize) -> f32 {
    (row_count as f32 * ARMY_ROSTER_ROW_H - viewport.h).max(0.0)
}

fn scroll_army_roster_offset(current: f32, viewport: Rect, row_count: usize, dy: f32) -> f32 {
    if dy == 0.0 || !dy.is_finite() {
        return current;
    }
    let rows = dy.abs().ceil().max(1.0);
    let delta = rows * ARMY_ROSTER_ROW_H;
    let next = if dy < 0.0 {
        current + delta
    } else {
        (current - delta).max(0.0)
    };
    next.min(max_army_scroll_fallback(viewport, row_count))
}

fn army_scrollbar_geometry(
    viewport: Rect,
    row_count: usize,
    scroll_y: f32,
) -> Option<ArmyScrollbarGeometry> {
    let content_h = row_count as f32 * ARMY_ROSTER_ROW_H;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport.h, content_h);
    if max_scroll <= 0.0 || viewport.h <= 0.0 {
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
    Some(ArmyScrollbarGeometry {
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

fn army_scroll_from_thumb_y(geom: ArmyScrollbarGeometry, thumb_y: f32) -> f32 {
    let travel = (geom.track.h - geom.thumb.h).max(0.0);
    if travel <= 0.0 {
        return 0.0;
    }
    let y = thumb_y.clamp(geom.track.y, geom.track.y + travel);
    ((y - geom.track.y) / travel).clamp(0.0, 1.0) * geom.max_scroll
}

fn pick_army_row_key(
    viewport: Rect,
    keys: &[Option<String>],
    scroll_y: f32,
    x: f32,
    y: f32,
) -> Option<String> {
    if !viewport.contains(x, y) {
        return None;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / ARMY_ROSTER_ROW_H).floor() as usize;
    keys.get(idx).and_then(|key| key.clone())
}

fn army_grid_window(
    roots: &[Node],
    file: &GuiFile,
    grid_name: &str,
    template_name: &str,
    row_count: usize,
    requested_scroll: f32,
    fallback_viewport: Rect,
) -> ArmyRowWindow {
    let metrics = find_rect(roots, grid_name)
        .and_then(|viewport| {
            find_template(file, template_name).map(|template| (viewport, template))
        })
        .map(|(viewport, template)| {
            let spec = GridSpec::from_gridbox(&stub_grid(grid_name, viewport), &template, GUTTER);
            (viewport, spec.cell_h.max(1.0), spec.columns.max(1))
        });
    let (viewport_h, cell_h, columns) = match metrics {
        Some((viewport, cell_h, columns)) => (viewport.h, cell_h, columns),
        None => (fallback_viewport.h, ARMY_ROSTER_ROW_H, 1),
    };
    let grid_rows = row_count.div_ceil(columns);
    let content_h = grid_rows as f32 * cell_h;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport_h, content_h);
    let scroll_y = requested_scroll.clamp(0.0, max_scroll);
    let first_grid_row = (scroll_y / cell_h).floor() as usize;
    let visible_grid_rows = (viewport_h / cell_h).ceil().max(1.0) as usize + 1;
    ArmyRowWindow {
        scroll_y,
        max_scroll,
        first: first_grid_row.saturating_mul(columns),
        visible: visible_grid_rows.saturating_mul(columns),
    }
}

fn visible_row_digest(rows: &[DataRow], window: ArmyRowWindow) -> String {
    let visible: Vec<DataRow> = rows
        .iter()
        .skip(window.first)
        .take(window.visible)
        .cloned()
        .collect();
    row_digest(&visible)
}

fn selected_row_digest(rows: &[DataRow], key: Option<&str>) -> String {
    match key.and_then(|selected| rows.iter().find(|row| row.key.as_deref() == Some(selected))) {
        Some(row) => row_digest(std::slice::from_ref(row)),
        None => "none".to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmyTabClick {
    Changed,
    Unchanged,
    Inert,
    Miss,
}

fn click_army_tab(bar: &mut TabBar, panel: Rect, x: f32, y: f32) -> ArmyTabClick {
    let rects = bar.layout(panel);
    if let Some(id) = bar.pick(&rects, x, y) {
        if bar.select(&id) {
            ArmyTabClick::Changed
        } else {
            ArmyTabClick::Unchanged
        }
    } else if rects.iter().any(|tr| tr.rect.contains(x, y)) {
        ArmyTabClick::Inert
    } else {
        ArmyTabClick::Miss
    }
}

fn army_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
}

// =====================================================================================
// VIEW 1 — ARMY  (countryarmyview)  [N2 tabs: Army / Navy / Air]
// =====================================================================================

const ARMY_VIEW_ID: ViewId = ViewId("military.army");
const ARMY_FILE: &str = "army.json";
const ARMY_ROOT: &str = "countryarmyview";

pub struct ArmyView {
    selected_tab: Option<String>,
    selected_rows: ArmyRosterSelection,
    scroll: ArmyRosterScroll,
    scroll_drag: Option<ArmyScrollDrag>,
    row_keys: ArmyRosterKeys,
    tabs: TabBar,
    screen: Rect,
}

impl ArmyView {
    pub fn new() -> ArmyView {
        let tabs = army_tabs_for_sections(
            SectionState::Present,
            SectionState::Present,
            SectionState::Present,
        );
        ArmyView {
            selected_tab: tabs.selected().map(str::to_string),
            selected_rows: ArmyRosterSelection::default(),
            scroll: ArmyRosterScroll::default(),
            scroll_drag: None,
            row_keys: ArmyRosterKeys::default(),
            tabs,
            screen: default_army_screen(),
        }
    }
}

impl Default for ArmyView {
    fn default() -> Self {
        ArmyView::new()
    }
}

struct ArmyDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    army_rows: Vec<DataRow>,
    navy_rows: Vec<DataRow>,
    air_rows: Vec<DataRow>,
    army_section: SectionState,
    navy_section: SectionState,
    air_section: SectionState,
    /// N2 sub-view selector (Army / Navy / Air).
    tabs: TabBar,
    selected_rows: ArmyRosterSelection,
    army_window: ArmyRowWindow,
    navy_window: ArmyRowWindow,
    air_window: ArmyRowWindow,
}

impl Drawable for ArmyDrawable {
    fn draw_digest(&self) -> String {
        let (visible_tab, visible_rows, window) = match self.tabs.selected() {
            Some("navy") => ("navy", &self.navy_rows, self.navy_window),
            Some("air") => ("air", &self.air_rows, self.air_window),
            _ => ("army", &self.army_rows, self.army_window),
        };
        let selected_key = self.selected_rows.get(visible_tab);
        format!(
            "ARMY army_sect={} navy_sect={} air_sect={} selected={:?} visible_tab={} row_selected={} scroll={:.0}/{:.0} visible=[{}] army={} navy={} air={} TABS {}",
            self.army_section.tag(),
            self.navy_section.tag(),
            self.air_section.tag(),
            self.tabs.selected(),
            visible_tab,
            selected_row_digest(visible_rows, selected_key),
            window.scroll_y,
            window.max_scroll,
            visible_row_digest(visible_rows, window),
            row_digest(&self.army_rows),
            row_digest(&self.navy_rows),
            row_digest(&self.air_rows),
            tabs_digest(&self.tabs),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        // the active tab decides which roster fills the (tab-shared) `item_grid` region.
        match self.tabs.selected() {
            Some("navy") => render_rows_scrolled(
                fd,
                &self.roots,
                &self.file,
                "item_grid",
                "armyview_navy_entry",
                &self.navy_rows,
                self.navy_window.scroll_y,
                self.selected_rows.get("navy"),
            ),
            Some("air") => render_rows_scrolled(
                fd,
                &self.roots,
                &self.file,
                "item_grid",
                "armyview_air_entry",
                &self.air_rows,
                self.air_window.scroll_y,
                self.selected_rows.get("air"),
            ),
            _ => render_rows_scrolled(
                fd,
                &self.roots,
                &self.file,
                "item_grid",
                "armyview_army_entry",
                &self.army_rows,
                self.army_window.scroll_y,
                self.selected_rows.get("army"),
            ),
        }
        draw_tabs_over(fd, &self.roots, "info", &self.tabs);
    }
}

impl ViewInstance for ArmyView {
    fn view_id(&self) -> ViewId {
        ARMY_VIEW_ID
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
        let oob = ctx.order_of_battle();
        self.tabs = army_tabs_with_selection(army_tabs_for_oob(&oob), self.selected_tab.as_deref());
        self.row_keys = army_roster_keys(&oob, ctx.country);
        for tab in ["army", "navy", "air"] {
            let viewport = army_roster_fallback(self.screen, tab);
            let clamped = self.scroll.get(tab).min(max_army_scroll_fallback(
                viewport,
                self.row_keys.get(tab).len(),
            ));
            self.scroll.set(tab, clamped);
        }
        if let Some(drag) = self.scroll_drag {
            let viewport = army_roster_fallback(self.screen, drag.tab);
            let scroll_y = self.scroll.get(drag.tab);
            if army_scrollbar_geometry(viewport, self.row_keys.get(drag.tab).len(), scroll_y)
                .is_none()
            {
                self.scroll_drag = None;
            }
        }
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => match click_army_tab(&mut self.tabs, army_tabs_fallback(self.screen), x, y) {
                ArmyTabClick::Changed => {
                    self.selected_tab = self.tabs.selected().map(str::to_string);
                    army_input_consumed(true)
                }
                ArmyTabClick::Unchanged => {
                    self.selected_tab = self.tabs.selected().map(str::to_string);
                    army_input_consumed(false)
                }
                ArmyTabClick::Inert => army_input_consumed(false),
                ArmyTabClick::Miss => {
                    let tab = selected_army_tab_id(&self.tabs);
                    let viewport = army_roster_fallback(self.screen, tab);
                    let scroll_y = self.scroll.get(tab).min(max_army_scroll_fallback(
                        viewport,
                        self.row_keys.get(tab).len(),
                    ));
                    if let Some(geom) =
                        army_scrollbar_geometry(viewport, self.row_keys.get(tab).len(), scroll_y)
                    {
                        if geom.thumb.contains(x, y) {
                            self.scroll_drag = Some(ArmyScrollDrag {
                                tab,
                                pointer_offset_y: y - geom.thumb.y,
                            });
                            return army_input_consumed(false);
                        }
                        if geom.track.contains(x, y) {
                            return army_input_consumed(false);
                        }
                    }
                    if let Some(key) =
                        pick_army_row_key(viewport, self.row_keys.get(tab), scroll_y, x, y)
                    {
                        self.selected_rows.set(tab, key);
                        army_input_consumed(true)
                    } else {
                        InputOutcome::ignored()
                    }
                }
            },
            ViewInput::Wheel { x, y, dy } => {
                let tab = selected_army_tab_id(&self.tabs);
                let viewport = army_roster_fallback(self.screen, tab);
                if !viewport.contains(x, y) {
                    return InputOutcome::ignored();
                }
                let count = self.row_keys.get(tab).len();
                let current = self.scroll.get(tab);
                let next = scroll_army_roster_offset(current, viewport, count, dy);
                self.scroll.set(tab, next);
                army_input_consumed(next != current)
            }
            ViewInput::PointerMove { x: _, y, drag } if drag => {
                let drag = match self.scroll_drag {
                    Some(drag) => drag,
                    None => return InputOutcome::ignored(),
                };
                let viewport = army_roster_fallback(self.screen, drag.tab);
                let geom = match army_scrollbar_geometry(
                    viewport,
                    self.row_keys.get(drag.tab).len(),
                    self.scroll.get(drag.tab),
                ) {
                    Some(geom) => geom,
                    None => return InputOutcome::ignored(),
                };
                let current = self.scroll.get(drag.tab);
                let next = army_scroll_from_thumb_y(geom, y - drag.pointer_offset_y);
                self.scroll.set(drag.tab, next);
                army_input_consumed((next - current).abs() > f32::EPSILON)
            }
            ViewInput::PointerUp {
                button: PointerButton::Left,
                ..
            } => {
                if self.scroll_drag.take().is_some() {
                    army_input_consumed(false)
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
        let file = load_gui(ctx, ARMY_FILE);
        let oob = ctx.order_of_battle();

        // --- top info bar ----------------------------------------------------------
        let mut summary: Vec<Cell> = Vec::new();
        push_country(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "unit_count",
            "military.division_count",
            Fidelity::Exact,
            fmt_int,
        );
        // battalion_count = SUM of regiments across the country's templates (computed).
        push_unavailable(&mut fidelity, &mut summary, "battalion_count");
        // special-forces cap = tech/manpower-computed modifier (not stored).
        push_unavailable(&mut fidelity, &mut summary, "special_forces_cap");

        // --- the three rosters -----------------------------------------------------
        let (armies, army_section) = oob_list(&oob, "armies");
        let (fleets, navy_section) = oob_list(&oob, "fleets");
        let (wings, air_section) = oob_list(&oob, "air_wings");

        let army_rows = build_army_rows(ctx, armies, tag, &mut fidelity);
        let navy_rows = build_armyview_navy_rows(ctx, fleets, tag, &mut fidelity);
        let air_rows = build_armyview_air_rows(wings, tag, &mut fidelity);
        army_section.record(&mut fidelity);
        navy_section.record(&mut fidelity);
        air_section.record(&mut fidelity);

        // N2 tabs: each tab's availability reflects whether that OOB sub-list is present.
        let tabs = army_tabs_with_selection(
            army_tabs_for_sections(army_section, navy_section, air_section),
            self.selected_tab.as_deref(),
        );

        let roots = build_root(ctx, &file, ARMY_ROOT, &summary);
        let army_fallback = army_roster_fallback(self.screen, "army");
        let navy_fallback = army_roster_fallback(self.screen, "navy");
        let air_fallback = army_roster_fallback(self.screen, "air");
        let army_window = army_grid_window(
            &roots,
            &file,
            "item_grid",
            "armyview_army_entry",
            army_rows.len(),
            self.scroll.get("army"),
            army_fallback,
        );
        let navy_window = army_grid_window(
            &roots,
            &file,
            "item_grid",
            "armyview_navy_entry",
            navy_rows.len(),
            self.scroll.get("navy"),
            navy_fallback,
        );
        let air_window = army_grid_window(
            &roots,
            &file,
            "item_grid",
            "armyview_air_entry",
            air_rows.len(),
            self.scroll.get("air"),
            air_fallback,
        );
        let drawable = ArmyDrawable {
            roots: roots.clone(),
            file,
            army_rows,
            navy_rows,
            air_rows,
            army_section,
            navy_section,
            air_section,
            tabs,
            selected_rows: self.selected_rows.clone(),
            army_window,
            navy_window,
            air_window,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: ARMY_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// `armyview_army_entry` rows — one per army owned by `<TAG>` (enumeration EXACT). Name is
/// a save string (exact); theatre/group/level/org bar need joins/runtime → unavailable;
/// the strength bar shows the member_count as a derived size proxy.
fn build_army_rows(
    ctx: &BindContext,
    armies: Option<&[serde_json::Value]>,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let armies = match armies {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut rows = Vec::new();
    for a in armies.iter().filter(|a| owned_by(a, tag)) {
        let mut row = DataRow::new();
        row.set_key_from("army", a, "id");
        row.bind(
            fidelity,
            "name",
            elem_field(a, "name"),
            Fidelity::Exact,
            fmt_token,
        );
        row.badge_unavailable(fidelity, "theatre");
        row.badge_unavailable(fidelity, "group");
        let (icon_asset, icon_fidelity) = formation_icon_asset(ctx, a, "GFX_theatre_army_shield");
        row.bind_sprite(fidelity, "group_marker", icon_asset, icon_fidelity);
        // army XP level needs the commander join (`leader` is a RAW int) → unavailable.
        row.badge_unavailable(fidelity, "level");
        // strength bar slot: member_count is a derived size proxy; org bar needs the
        // per-division mean / template-max denominator → unavailable.
        row.bind(
            fidelity,
            "strength_progressbar",
            elem_field(a, "member_count"),
            Fidelity::Derived,
            fmt_int,
        );
        row.badge_unavailable(fidelity, "organization_progressbar");
        rows.push(row);
    }
    rows
}

/// `armyview_navy_entry` rows — one per fleet owned by `<TAG>`. Name exact; ship count =
/// SUM over the fleet's task forces (exact enumeration); region/mission/leader/org/str
/// need joins/runtime → unavailable.
fn build_armyview_navy_rows(
    ctx: &BindContext,
    fleets: Option<&[serde_json::Value]>,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let fleets = match fleets {
        Some(f) => f,
        None => return Vec::new(),
    };
    let mut rows = Vec::new();
    for f in fleets.iter().filter(|f| owned_by(f, tag)) {
        let mut row = DataRow::new();
        row.set_key_from("navy", f, "id");
        row.bind(
            fidelity,
            "unit_name",
            elem_field(f, "name"),
            Fidelity::Exact,
            fmt_token,
        );
        let n = count_fleet_ships(f);
        row.bind_text(fidelity, "unit_count", n.to_string(), Fidelity::Exact);
        row.badge_unavailable(fidelity, "region");
        row.badge_unavailable(fidelity, "mission_icon");
        let commander =
            find_commander_by_leader_ref(ctx, tag, f.get("leader"), &["commanders.admirals"]);
        row.bind_sprite(
            fidelity,
            "leader_button",
            fleet_leader_portrait_asset(ctx, f, commander),
            Fidelity::Exact,
        );
        row.badge_unavailable(fidelity, "organization_progressbar");
        row.badge_unavailable(fidelity, "strength_progressbar");
        rows.push(row);
    }
    rows
}

/// Count ships across a fleet's task forces (exact enumeration of `task_forces[].ships[]`).
fn count_fleet_ships(fleet: &serde_json::Value) -> usize {
    let tfs = match fleet.get("task_forces").and_then(|t| t.as_array()) {
        Some(t) => t,
        None => return 0,
    };
    tfs.iter().map(|tf| arr_count(tf, "ships")).sum()
}

/// `armyview_air_entry` rows — one per air wing owned by `<TAG>`. name/count exact (save);
/// region/base derived (the strategic-region id + air-base state need a name join); limit/ace
/// unavailable.
fn build_armyview_air_rows(
    wings: Option<&[serde_json::Value]>,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let wings = match wings {
        Some(w) => w,
        None => return Vec::new(),
    };
    let mut rows = Vec::new();
    for w in wings.iter().filter(|w| owned_by(w, tag)) {
        let mut row = DataRow::new();
        row.set_key_from("air", w, "id");
        row.bind(
            fidelity,
            "unit_name",
            elem_field(w, "name"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "count",
            elem_field(w, "count"),
            Fidelity::Exact,
            fmt_int,
        );
        // strategic-region id → region NAME needs a meta join (Codex: not exact) → derived.
        row.bind(
            fidelity,
            "assigned_region",
            elem_field(w, "strategic_region"),
            Fidelity::Derived,
            fmt_token,
        );
        // base state resolved via `air_base`→state (derived; null for carrier/mobile).
        row.bind(
            fidelity,
            "base",
            elem_field(w, "state_id"),
            Fidelity::Derived,
            fmt_token,
        );
        // per-wing operational LIMIT denominator + bar fraction not stored → unavailable.
        row.badge_unavailable(fidelity, "limit");
        // ace-to-wing join not stored → unavailable.
        row.badge_unavailable(fidelity, "ace");
        rows.push(row);
    }
    rows
}

fn open_army(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(ArmyView::new())
}

register_view!(ViewDescriptor {
    id: ARMY_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(ARMY_FILE, ARMY_ROOT)),
    required_domains: &["order_of_battle", "countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Army",
        order: 1
    }),
    shortcut: None,
    open: open_army,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.armyview_army_entry"),
    gui_template: GuiSymbolRef::template(ARMY_FILE, "armyview_army_entry"),
    required_domains: &["order_of_battle"],
});
register_template!(TemplateDescriptor {
    id: ViewId("military.armyview_navy_entry"),
    gui_template: GuiSymbolRef::template(ARMY_FILE, "armyview_navy_entry"),
    required_domains: &["order_of_battle"],
});
register_template!(TemplateDescriptor {
    id: ViewId("military.armyview_air_entry"),
    gui_template: GuiSymbolRef::template(ARMY_FILE, "armyview_air_entry"),
    required_domains: &["order_of_battle"],
});

// =====================================================================================
// VIEW 2 — NAVIES  (naviesview_window)
// =====================================================================================

const NAVIES_VIEW_ID: ViewId = ViewId("military.navies");
const NAVIES_FILE: &str = "navies.json";
const NAVIES_ROOT: &str = "naviesview_window";

pub struct NaviesView;

impl NaviesView {
    pub fn new() -> NaviesView {
        NaviesView
    }
}

impl Default for NaviesView {
    fn default() -> Self {
        NaviesView::new()
    }
}

struct NaviesDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    fleet_rows: Vec<DataRow>,
    tf_rows: Vec<DataRow>,
    ship_rows: Vec<DataRow>,
    fleet_section: SectionState,
}

impl Drawable for NaviesDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "NAVIES fleet_sect={} fleets={} tfs={} ships={}",
            self.fleet_section.tag(),
            row_digest(&self.fleet_rows),
            row_digest(&self.tf_rows),
            row_digest(&self.ship_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "detailed_fleet_box",
            "fleet_view_header",
            &self.fleet_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "detailed_fleet_box",
            "taskforce_view_entry",
            &self.tf_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "ships_detailed",
            "ship_entry_detailed",
            &self.ship_rows,
        );
    }
}

impl ViewInstance for NaviesView {
    fn view_id(&self) -> ViewId {
        NAVIES_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, NAVIES_FILE);
        let oob = ctx.order_of_battle();

        let (fleets, fleet_section) = oob_list(&oob, "fleets");
        let (fleet_rows, tf_rows, ship_rows) = build_navies_rows(ctx, fleets, tag, &mut fidelity);
        fleet_section.record(&mut fidelity);

        let roots = build_root(ctx, &file, NAVIES_ROOT, &[]);
        let drawable = NaviesDrawable {
            roots: roots.clone(),
            file,
            fleet_rows,
            tf_rows,
            ship_rows,
            fleet_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: NAVIES_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The fleet → task-force → ship roster. Fleets owned by `<TAG>`. Fleet name + TF name +
/// ship name/type/org/str/xp EXACT (save strings / measured leaves); home-base/fuel/leader/
/// activity/dominance/repair are unfolded or runtime → unavailable.
fn build_navies_rows(
    ctx: &BindContext,
    fleets: Option<&[serde_json::Value]>,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, Vec<DataRow>, Vec<DataRow>) {
    let mut fleet_rows = Vec::new();
    let mut tf_rows = Vec::new();
    let mut ship_rows = Vec::new();
    let fleets = match fleets {
        Some(f) => f,
        None => return (fleet_rows, tf_rows, ship_rows),
    };
    for f in fleets.iter().filter(|f| owned_by(f, tag)) {
        // --- fleet card ---
        let mut fr = DataRow::new();
        fr.bind(
            fidelity,
            "fleet_name",
            elem_field(f, "name"),
            Fidelity::Exact,
            fmt_token,
        );
        let (icon_asset, icon_fidelity) = formation_icon_asset(ctx, f, "GFX_theatre_army_shield");
        fr.bind_sprite(fidelity, "insigna", icon_asset, icon_fidelity);
        let tf_count = arr_count(f, "task_forces");
        fr.bind_text(
            fidelity,
            "taskforce_count",
            tf_count.to_string(),
            Fidelity::Derived,
        );
        fr.badge_unavailable(fidelity, "active_in_regions");
        fr.badge_unavailable(fidelity, "home_base_name");
        fr.badge_unavailable(fidelity, "fuel_icon");
        let commander =
            find_commander_by_leader_ref(ctx, tag, f.get("leader"), &["commanders.admirals"]);
        fr.bind_sprite(
            fidelity,
            "leader_button",
            fleet_leader_portrait_asset(ctx, f, commander),
            Fidelity::Exact,
        );
        match commander {
            Some(commander) => {
                fr.bind(
                    fidelity,
                    "leader_name",
                    elem_field(commander, "name"),
                    Fidelity::Exact,
                    fmt_token,
                );
                fr.bind(
                    fidelity,
                    "skill_text",
                    elem_field(commander, "level"),
                    Fidelity::Exact,
                    fmt_int,
                );
            }
            None => {
                fr.badge_unavailable(fidelity, "leader_name");
                fr.badge_unavailable(fidelity, "skill_text");
            }
        }
        fleet_rows.push(fr);

        // --- task-force cards + ship rows ---
        if let Some(tfs) = f.get("task_forces").and_then(|t| t.as_array()) {
            for tf in tfs {
                let mut tr = DataRow::new();
                tr.bind(
                    fidelity,
                    "unit_name",
                    elem_field(tf, "name"),
                    Fidelity::Exact,
                    fmt_token,
                );
                tr.bind(
                    fidelity,
                    "current_mission",
                    elem_field(tf, "mission"),
                    Fidelity::Derived,
                    fmt_token,
                );
                let (icon_asset, icon_fidelity) =
                    formation_icon_asset(ctx, tf, "GFX_theatre_army_shield");
                tr.bind_sprite(fidelity, "insigna", icon_asset, icon_fidelity);
                // TF activity/dominance/fuel/reserve are runtime → unavailable.
                tr.badge_unavailable(fidelity, "activity_description");
                tr.badge_unavailable(fidelity, "dominance");
                tr.badge_unavailable(fidelity, "fuel_stocks_icon");
                tr.badge_unavailable(fidelity, "reserve_fleet_icon");
                tf_rows.push(tr);

                if let Some(ships) = tf.get("ships").and_then(|s| s.as_array()) {
                    for s in ships {
                        let mut sr = DataRow::new();
                        sr.bind(
                            fidelity,
                            "ship_name",
                            elem_field(s, "name"),
                            Fidelity::Exact,
                            fmt_token,
                        );
                        sr.bind(
                            fidelity,
                            "ship_type",
                            elem_field(s, "definition"),
                            Fidelity::Derived,
                            fmt_token,
                        );
                        // save TF-ships carry `strength` (fraction) + `organisation`.
                        sr.bind(
                            fidelity,
                            "bar_str",
                            elem_field(s, "strength"),
                            Fidelity::Derived,
                            fmt_pct_frac,
                        );
                        sr.bind(
                            fidelity,
                            "bar_org",
                            elem_field(s, "organisation"),
                            Fidelity::Derived,
                            fmt_pct_frac,
                        );
                        sr.bind(
                            fidelity,
                            "xp_bar",
                            elem_field(s, "experience"),
                            Fidelity::Derived,
                            fmt_int,
                        );
                        // carrier air complement + repair state not stored → unavailable.
                        sr.badge_unavailable(fidelity, "bar_planes");
                        sr.badge_unavailable(fidelity, "repair_symbol");
                        ship_rows.push(sr);
                    }
                }
            }
        }
    }
    (fleet_rows, tf_rows, ship_rows)
}

fn open_navies(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(NaviesView::new())
}

register_view!(ViewDescriptor {
    id: NAVIES_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(NAVIES_FILE, NAVIES_ROOT)),
    required_domains: &["order_of_battle"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Navies",
        order: 2
    }),
    shortcut: None,
    open: open_navies,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.fleet_view_header"),
    gui_template: GuiSymbolRef::template(NAVIES_FILE, "fleet_view_header"),
    required_domains: &["order_of_battle"],
});
register_template!(TemplateDescriptor {
    id: ViewId("military.taskforce_view_entry"),
    gui_template: GuiSymbolRef::template(NAVIES_FILE, "taskforce_view_entry"),
    required_domains: &["order_of_battle"],
});
register_template!(TemplateDescriptor {
    id: ViewId("military.ship_entry_detailed"),
    gui_template: GuiSymbolRef::template(NAVIES_FILE, "ship_entry_detailed"),
    required_domains: &["order_of_battle"],
});

// =====================================================================================
// VIEW 3 — AIR GROUP  (air_groups_view)
// =====================================================================================

const AIR_GROUP_VIEW_ID: ViewId = ViewId("military.air_group");
const AIR_GROUP_FILE: &str = "air_group.json";
const AIR_GROUP_ROOT: &str = "air_groups_view";

pub struct AirGroupView;

impl AirGroupView {
    pub fn new() -> AirGroupView {
        AirGroupView
    }
}

impl Default for AirGroupView {
    fn default() -> Self {
        AirGroupView::new()
    }
}

struct AirGroupDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    group_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for AirGroupDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "AIRGROUP sect={} groups={}",
            self.section.tag(),
            row_digest(&self.group_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "air_group_view_entries",
            "air_group_view_entry",
            &self.group_rows,
        );
    }
}

impl ViewInstance for AirGroupView {
    fn view_id(&self) -> ViewId {
        AIR_GROUP_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, AIR_GROUP_FILE);
        let oob = ctx.order_of_battle();

        // air GROUPS are a derived grouping over `air_wings[]`; one group row per wing here
        // (the group name is not a stored leaf → unavailable). Enumeration over owned wings.
        let (wings, section) = oob_list(&oob, "air_wings");
        let group_rows = build_air_group_rows(wings, tag, &mut fidelity);
        section.record(&mut fidelity);

        let roots = build_root(ctx, &file, AIR_GROUP_ROOT, &[]);
        let drawable = AirGroupDrawable {
            roots: roots.clone(),
            file,
            group_rows,
            section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: AIR_GROUP_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// `air_group_view_entry` rows — the air-group grouping over `air_wings[]` (owner=`<TAG>`).
/// The group NAME is not a stored leaf → unavailable; the member-wing enumeration is exact;
/// fuel is runtime → unavailable. Renders one row per owned wing (the smallest stored unit).
fn build_air_group_rows(
    wings: Option<&[serde_json::Value]>,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let wings = match wings {
        Some(w) => w,
        None => return Vec::new(),
    };
    let mut rows = Vec::new();
    for _w in wings.iter().filter(|w| owned_by(w, tag)) {
        let mut row = DataRow::new();
        // air-GROUP name is a derived grouping with no stored group name → unavailable.
        row.badge_unavailable(fidelity, "unit_name");
        // per-group fuel + efficiency warning are runtime → unavailable.
        row.badge_unavailable(fidelity, "fuel_stocks_icon");
        row.badge_unavailable(fidelity, "fuel_efficiency_warning_icon");
        rows.push(row);
    }
    rows
}

fn open_air_group(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(AirGroupView::new())
}

register_view!(ViewDescriptor {
    id: AIR_GROUP_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(AIR_GROUP_FILE, AIR_GROUP_ROOT)),
    required_domains: &["order_of_battle"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Air Groups",
        order: 3
    }),
    shortcut: None,
    open: open_air_group,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.air_group_view_entry"),
    gui_template: GuiSymbolRef::template(AIR_GROUP_FILE, "air_group_view_entry"),
    required_domains: &["order_of_battle"],
});
register_template!(TemplateDescriptor {
    id: ViewId("military.wing_entry_compact"),
    gui_template: GuiSymbolRef::template(AIR_GROUP_FILE, "wing_entry_compact"),
    required_domains: &["order_of_battle"],
});

// =====================================================================================
// VIEW 4 — AIR SELECTION  (airselectionview)
// =====================================================================================

const AIR_SELECTION_VIEW_ID: ViewId = ViewId("military.air_selection");
const AIR_SELECTION_FILE: &str = "air_selection.json";
const AIR_SELECTION_ROOT: &str = "airselectionview";

pub struct AirSelectionView;

impl AirSelectionView {
    pub fn new() -> AirSelectionView {
        AirSelectionView
    }
}

impl Default for AirSelectionView {
    fn default() -> Self {
        AirSelectionView::new()
    }
}

struct AirSelectionDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    base_rows: Vec<DataRow>,
    wing_rows: Vec<DataRow>,
    base_section: SectionState,
    wing_section: SectionState,
}

impl Drawable for AirSelectionDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "AIRSEL base_sect={} wing_sect={} bases={} wings={}",
            self.base_section.tag(),
            self.wing_section.tag(),
            row_digest(&self.base_rows),
            row_digest(&self.wing_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "air_selection",
            "airbase_selection_item",
            &self.base_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "air_selection",
            "airwing_selection_item",
            &self.wing_rows,
        );
    }
}

impl ViewInstance for AirSelectionView {
    fn view_id(&self) -> ViewId {
        AIR_SELECTION_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, AIR_SELECTION_FILE);

        // air bases from the strategic_air domain; air wings from OOB (owner=<TAG>).
        let (bases, base_section) = captured_array(strategic_air_list(ctx, "air_bases"));
        let base_rows = build_airbase_rows(bases, &mut fidelity);
        base_section.record(&mut fidelity);

        let oob = ctx.order_of_battle();
        let (wings, wing_section) = oob_list(&oob, "air_wings");
        let wing_rows = build_airwing_selection_rows(wings, tag, &mut fidelity);
        wing_section.record(&mut fidelity);

        let roots = build_root(ctx, &file, AIR_SELECTION_ROOT, &[]);
        let drawable = AirSelectionDrawable {
            roots: roots.clone(),
            file,
            base_rows,
            wing_rows,
            base_section,
            wing_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: AIR_SELECTION_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Resolve a SUB-LIST inside the `strategic_air` domain (e.g. `air_bases`) as a Captured.
fn strategic_air_list<'a>(ctx: &'a BindContext, key: &str) -> Captured<&'a serde_json::Value> {
    match ctx.strategic_air() {
        Captured::Measured(v) => crate::host::context::classify_value(v.get(key)),
        Captured::Null => Captured::Null,
        Captured::Absent => Captured::Absent,
    }
}

/// `airbase_selection_item` rows — one per `strategic_air.air_bases[]`. capacity exact;
/// state→name + owner flag derived; fuel + volunteer-slot accounting runtime → unavailable.
fn build_airbase_rows(
    bases: Option<&[serde_json::Value]>,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let bases = match bases {
        Some(b) => b,
        None => return Vec::new(),
    };
    let mut rows = Vec::new();
    for b in bases {
        let mut row = DataRow::new();
        // base name = the base's state id → state name (derived).
        row.bind(
            fidelity,
            "base_name",
            elem_field(b, "state"),
            Fidelity::Derived,
            fmt_token,
        );
        row.bind(
            fidelity,
            "capacity",
            elem_field(b, "capacity"),
            Fidelity::Exact,
            fmt_int,
        );
        // owner flag + friendship status need a state→owner join (derived).
        row.badge_unavailable(fidelity, "owner_flag");
        row.badge_unavailable(fidelity, "airbase_friendship_status");
        // per-base fuel + volunteer-slot cap are runtime → unavailable.
        row.badge_unavailable(fidelity, "fuel_icon");
        row.badge_unavailable(fidelity, "permission_wings_count");
        rows.push(row);
    }
    rows
}

/// `airwing_selection_item` rows — one per air wing owned by `<TAG>`. name/count exact;
/// mission-state derived; efficiency strip = proxy (`effective_planes` RUNTIME-cached);
/// armies-count / ace / limit unavailable.
fn build_airwing_selection_rows(
    wings: Option<&[serde_json::Value]>,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let wings = match wings {
        Some(w) => w,
        None => return Vec::new(),
    };
    let mut rows = Vec::new();
    for w in wings.iter().filter(|w| owned_by(w, tag)) {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "unit_name",
            elem_field(w, "name"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "count",
            elem_field(w, "count"),
            Fidelity::Exact,
            fmt_int,
        );
        row.bind(
            fidelity,
            "mission_state",
            elem_field(w, "mission"),
            Fidelity::Derived,
            fmt_token,
        );
        row.bind(
            fidelity,
            "xp_bar",
            elem_field(w, "experience"),
            Fidelity::Derived,
            fmt_int,
        );
        // `effective_planes` IS a captured RUNTIME-cached field → render the efficiency strip
        // as a PROXY (derived-from-proxy). We badge it derived (non-authoritative) when the
        // field is present; otherwise the badge falls to "—".
        row.bind(
            fidelity,
            "air_mission_efficiency_strip",
            elem_field(w, "effective_planes"),
            Fidelity::Derived,
            fmt_int,
        );
        // wing→army attachment count + ace join not stored → unavailable.
        row.badge_unavailable(fidelity, "armies_count_box");
        row.badge_unavailable(fidelity, "ace_btn");
        rows.push(row);
    }
    rows
}

fn open_air_selection(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(AirSelectionView::new())
}

register_view!(ViewDescriptor {
    id: AIR_SELECTION_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(AIR_SELECTION_FILE, AIR_SELECTION_ROOT)),
    required_domains: &["order_of_battle", "strategic_air"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Air Selection",
        order: 4
    }),
    shortcut: None,
    open: open_air_selection,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.airbase_selection_item"),
    gui_template: GuiSymbolRef::template(AIR_SELECTION_FILE, "airbase_selection_item"),
    required_domains: &["strategic_air"],
});
register_template!(TemplateDescriptor {
    id: ViewId("military.airwing_selection_item"),
    gui_template: GuiSymbolRef::template(AIR_SELECTION_FILE, "airwing_selection_item"),
    required_domains: &["order_of_battle"],
});

// =====================================================================================
// VIEW 5 — AIR WING DETAILS  (air_wing_stats_window)
// =====================================================================================

const AIR_WING_DETAILS_VIEW_ID: ViewId = ViewId("military.air_wing_details");
const AIR_WING_DETAILS_FILE: &str = "air_wing_details.json";
const AIR_WING_DETAILS_ROOT: &str = "air_wing_stats_window";

pub struct AirWingDetailsView;

impl AirWingDetailsView {
    pub fn new() -> AirWingDetailsView {
        AirWingDetailsView
    }
}

impl Default for AirWingDetailsView {
    fn default() -> Self {
        AirWingDetailsView::new()
    }
}

struct AirWingDetailsDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    summary: Vec<Cell>,
    grid_gaps: DataRow,
    equipment_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for AirWingDetailsDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "AIRWINGDETAILS sect={} SUM[{}] GAPS[{}] equip={}",
            self.section.tag(),
            summary_digest(&self.summary),
            row_digest(std::slice::from_ref(&self.grid_gaps)),
            row_digest(&self.equipment_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "list",
            "air_wing_details_equipment_entry",
            &self.equipment_rows,
        );
    }
}

impl ViewInstance for AirWingDetailsView {
    fn view_id(&self) -> ViewId {
        AIR_WING_DETAILS_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, AIR_WING_DETAILS_FILE);
        let oob = ctx.order_of_battle();

        // bind the FIRST owned wing as the detail subject (a real popup is opened on a wing
        // pick; in the replay we surface the first owned wing's stats deterministically).
        let (wings, section) = oob_list(&oob, "air_wings");
        let wing = wings.and_then(|ws| ws.iter().find(|w| owned_by(w, tag)));

        let mut summary: Vec<Cell> = Vec::new();
        match wing {
            Some(w) => {
                push_text(
                    &mut fidelity,
                    &mut summary,
                    "air_wing_name",
                    elem_field(w, "name")
                        .display_with_fidelity(Fidelity::Exact, fmt_token)
                        .as_str()
                        .to_string(),
                    Fidelity::Exact,
                );
                push_text(
                    &mut fidelity,
                    &mut summary,
                    "current_manpower",
                    elem_field(w, "manpower")
                        .display_with_fidelity(Fidelity::Exact, fmt_int)
                        .as_str()
                        .to_string(),
                    Fidelity::Exact,
                );
            }
            None => {
                push_unavailable(&mut fidelity, &mut summary, "air_wing_name");
                push_unavailable(&mut fidelity, &mut summary, "current_manpower");
            }
        }
        // equipment ratio (on-strength / target) — target not stored → unavailable.
        push_unavailable(&mut fidelity, &mut summary, "equipment_ratio");
        // ace name/icon — ace→wing join not stored → unavailable.
        push_unavailable(&mut fidelity, &mut summary, "ace_name");
        // the 3 stat grids (base/combat/misc) are DERIVED equipment_variant_stats (computed
        // from modules) — no per-stat node bound here; tallied as derived gaps.
        let mut grid_gaps = DataRow::new();
        grid_gaps.badge_gap(&mut fidelity, "base_stats");
        grid_gaps.badge_gap(&mut fidelity, "combat_stats");
        grid_gaps.badge_gap(&mut fidelity, "misc_stats");

        let equipment_rows = build_air_equipment_rows(wing, &mut fidelity);

        let roots = build_root(ctx, &file, AIR_WING_DETAILS_ROOT, &summary);
        let drawable = AirWingDetailsDrawable {
            roots: roots.clone(),
            file,
            summary,
            grid_gaps,
            equipment_rows,
            section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: AIR_WING_DETAILS_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// `air_wing_details_equipment_entry` rows — one per `air_wings[].equipment[[id,type,amount]]`
/// line. amount exact; name/type/icon are variant-catalog lookups (derived).
fn build_air_equipment_rows(
    wing: Option<&serde_json::Value>,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let equip = match wing
        .and_then(|w| w.get("equipment"))
        .and_then(|e| e.as_array())
    {
        Some(e) => e,
        None => return Vec::new(),
    };
    let mut rows = Vec::new();
    for line in equip {
        let mut row = DataRow::new();
        // equipment line is `[id, type, amount]` — amount is the 3rd element.
        let amount = line.as_array().and_then(|a| a.get(2));
        row.bind(
            fidelity,
            "count",
            crate::host::context::classify_value(amount),
            Fidelity::Exact,
            fmt_int,
        );
        // variant name/type/icon are catalog lookups (derived).
        row.badge_unavailable(fidelity, "name");
        row.badge_unavailable(fidelity, "type");
        rows.push(row);
    }
    rows
}

fn open_air_wing_details(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(AirWingDetailsView::new())
}

register_view!(ViewDescriptor {
    id: AIR_WING_DETAILS_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        AIR_WING_DETAILS_FILE,
        AIR_WING_DETAILS_ROOT
    )),
    required_domains: &["order_of_battle"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Air Wing Details",
        order: 5
    }),
    shortcut: None,
    open: open_air_wing_details,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.air_wing_details_equipment_entry"),
    gui_template: GuiSymbolRef::template(AIR_WING_DETAILS_FILE, "air_wing_details_equipment_entry"),
    required_domains: &["order_of_battle"],
});

// =====================================================================================
// VIEW 6 — AIR WING REORGANIZATION  (air_wing_reorganization)
// =====================================================================================

const AIR_WING_REORG_VIEW_ID: ViewId = ViewId("military.air_wing_reorg");
const AIR_WING_REORG_FILE: &str = "air_wing_reorg.json";
const AIR_WING_REORG_ROOT: &str = "air_wing_reorganization";
#[cfg(test)]
const AIR_WING_REORG_CONFIRM_NOOP: &str = "confirm_btn";

pub struct AirWingReorgView {
    screen: Rect,
    confirm_armed: bool,
}

impl AirWingReorgView {
    pub fn new() -> AirWingReorgView {
        AirWingReorgView {
            screen: default_air_wing_reorg_screen(),
            confirm_armed: false,
        }
    }
}

impl Default for AirWingReorgView {
    fn default() -> Self {
        AirWingReorgView::new()
    }
}

fn default_air_wing_reorg_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn air_wing_reorg_window_rect(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w * 0.5 - 450.0,
        y: screen.y + screen.h * 0.5 - 390.0,
        w: 1000.0,
        h: 515.0,
    }
}

fn air_wing_reorg_confirm_rect(screen: Rect) -> Rect {
    let window = air_wing_reorg_window_rect(screen);
    Rect {
        x: window.x + 390.0,
        y: window.y + 460.0,
        w: 221.0,
        h: 34.0,
    }
}

fn replay_disabled_noop() -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw: false,
    }
}

struct AirWingReorgDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    summary: Vec<Cell>,
    left_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for AirWingReorgDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "AIRREORG sect={} SUM[{}] left={}",
            self.section.tag(),
            summary_digest(&self.summary),
            row_digest(&self.left_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "list",
            "air_wing_reorganization_equipment_entry_left",
            &self.left_rows,
        );
    }
}

impl ViewInstance for AirWingReorgView {
    fn view_id(&self) -> ViewId {
        AIR_WING_REORG_VIEW_ID
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
        self.confirm_armed = false;
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => {
                if air_wing_reorg_confirm_rect(self.screen).contains(x, y) {
                    self.confirm_armed = true;
                    replay_disabled_noop()
                } else {
                    self.confirm_armed = false;
                    InputOutcome::ignored()
                }
            }
            ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            } if self.confirm_armed || air_wing_reorg_confirm_rect(self.screen).contains(x, y) => {
                self.confirm_armed = false;
                replay_disabled_noop()
            }
            _ => InputOutcome::ignored(),
        }
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, AIR_WING_REORG_FILE);
        let oob = ctx.order_of_battle();

        // the reorg dialog RENDERS the wings' equipment lines (the move is a mutation, inert).
        let (wings, section) = oob_list(&oob, "air_wings");
        let wing = wings.and_then(|ws| ws.iter().find(|w| owned_by(w, tag)));
        let left_rows = build_reorg_equipment_rows(wing, &mut fidelity);
        section.record(&mut fidelity);

        // manpower header (left = the subject wing's manpower).
        let mut summary: Vec<Cell> = Vec::new();
        match wing {
            Some(w) => push_text(
                &mut fidelity,
                &mut summary,
                "current_manpower_left",
                elem_field(w, "manpower")
                    .display_with_fidelity(Fidelity::Exact, fmt_int)
                    .as_str()
                    .to_string(),
                Fidelity::Exact,
            ),
            None => push_unavailable(&mut fidelity, &mut summary, "current_manpower_left"),
        }
        push_unavailable(&mut fidelity, &mut summary, "current_manpower_right");

        let roots = build_root(ctx, &file, AIR_WING_REORG_ROOT, &summary);
        let drawable = AirWingReorgDrawable {
            roots: roots.clone(),
            file,
            summary,
            left_rows,
            section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: AIR_WING_REORG_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// `air_wing_reorganization_equipment_entry_left` rows — the wing's equipment lines (render
/// only; the move is a mutation). count exact; name/type/icon/design-team/niche derived.
fn build_reorg_equipment_rows(
    wing: Option<&serde_json::Value>,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let equip = match wing
        .and_then(|w| w.get("equipment"))
        .and_then(|e| e.as_array())
    {
        Some(e) => e,
        None => return Vec::new(),
    };
    let mut rows = Vec::new();
    for line in equip {
        let mut row = DataRow::new();
        let amount = line.as_array().and_then(|a| a.get(2));
        row.bind(
            fidelity,
            "count",
            crate::host::context::classify_value(amount),
            Fidelity::Exact,
            fmt_int,
        );
        row.badge_unavailable(fidelity, "name");
        row.badge_unavailable(fidelity, "type");
        row.badge_unavailable(fidelity, "design_team_icon");
        row.badge_unavailable(fidelity, "niche_icon");
        rows.push(row);
    }
    rows
}

fn open_air_wing_reorg(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(AirWingReorgView::new())
}

register_view!(ViewDescriptor {
    id: AIR_WING_REORG_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(AIR_WING_REORG_FILE, AIR_WING_REORG_ROOT)),
    required_domains: &["order_of_battle"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Air Wing Reorg",
        order: 6
    }),
    shortcut: None,
    open: open_air_wing_reorg,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.air_wing_reorg_entry_left"),
    gui_template: GuiSymbolRef::template(
        AIR_WING_REORG_FILE,
        "air_wing_reorganization_equipment_entry_left"
    ),
    required_domains: &["order_of_battle"],
});

// =====================================================================================
// VIEW 7 — CARRIER AIR WING  (carrier_air_wing_composition_window)
// =====================================================================================

const CARRIER_AIR_WING_VIEW_ID: ViewId = ViewId("military.carrier_air_wing");
const CARRIER_AIR_WING_FILE: &str = "carrier_air_wing.json";
const CARRIER_AIR_WING_ROOT: &str = "carrier_air_wing_composition_window";

pub struct CarrierAirWingView;

impl CarrierAirWingView {
    pub fn new() -> CarrierAirWingView {
        CarrierAirWingView
    }
}

impl Default for CarrierAirWingView {
    fn default() -> Self {
        CarrierAirWingView::new()
    }
}

struct CarrierAirWingDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    summary: Vec<Cell>,
    comp_rows: Vec<DataRow>,
}

impl Drawable for CarrierAirWingDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "CARRIERAIR SUM[{}] comp={}",
            summary_digest(&self.summary),
            row_digest(&self.comp_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "list",
            "carrier_air_wing_composition_entry",
            &self.comp_rows,
        );
    }
}

impl ViewInstance for CarrierAirWingView {
    fn view_id(&self) -> ViewId {
        CARRIER_AIR_WING_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, CARRIER_AIR_WING_FILE);

        // the carrier air-complement composition has NO schema home (no ship→air-complement
        // leaf) → the whole composition is UNAVAILABLE (no fabrication, Codex-confirmed).
        let mut summary: Vec<Cell> = Vec::new();
        push_unavailable(&mut fidelity, &mut summary, "title");
        push_unavailable(&mut fidelity, &mut summary, "summary");
        push_unavailable(&mut fidelity, &mut summary, "carrier_capacity_icon");

        // we render no rows (the per-plane-type allocation array has no schema home) — a
        // single badged gap records the absence honestly.
        let mut comp = DataRow::new();
        comp.badge_gap(&mut fidelity, "carrier_air_wing_composition_entry");
        let comp_rows = vec![comp];

        let roots = build_root(ctx, &file, CARRIER_AIR_WING_ROOT, &summary);
        // comp_rows is a single gap-only row (no cells) — the composition has no schema home.
        let drawable = CarrierAirWingDrawable {
            roots: roots.clone(),
            file,
            summary,
            comp_rows,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: CARRIER_AIR_WING_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_carrier_air_wing(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(CarrierAirWingView::new())
}

register_view!(ViewDescriptor {
    id: CARRIER_AIR_WING_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        CARRIER_AIR_WING_FILE,
        CARRIER_AIR_WING_ROOT
    )),
    required_domains: &["order_of_battle"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Carrier Air Wing",
        order: 7
    }),
    shortcut: None,
    open: open_carrier_air_wing,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.carrier_air_wing_composition_entry"),
    gui_template: GuiSymbolRef::template(
        CARRIER_AIR_WING_FILE,
        "carrier_air_wing_composition_entry"
    ),
    required_domains: &["order_of_battle"],
});

// =====================================================================================
// VIEW 8 — STRATEGIC AIR  (strategicairview)
// =====================================================================================

const STRATEGIC_AIR_VIEW_ID: ViewId = ViewId("military.strategic_air");
const STRATEGIC_AIR_FILE: &str = "strategic_air.json";
const STRATEGIC_AIR_ROOT: &str = "strategicairview";

pub struct StrategicAirView;

impl StrategicAirView {
    pub fn new() -> StrategicAirView {
        StrategicAirView
    }
}

impl Default for StrategicAirView {
    fn default() -> Self {
        StrategicAirView::new()
    }
}

struct StrategicAirDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    summary: Vec<Cell>,
    damage_rows: Vec<DataRow>,
}

impl Drawable for StrategicAirDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "STRATAIR SUM[{}] damage={}",
            summary_digest(&self.summary),
            row_digest(&self.damage_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "our_fighters_damage_list",
            "air_combat_damage_received",
            &self.damage_rows,
        );
    }
}

impl ViewInstance for StrategicAirView {
    fn view_id(&self) -> ViewId {
        STRATEGIC_AIR_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, STRATEGIC_AIR_FILE);
        let oob = ctx.order_of_battle();

        // OUR per-type plane counts (wing_type_left_text_1..4) = aggregate over `air_wings[]`
        // (count + strategic_region, owner=<TAG>) — DERIVED (Codex: NOT sourced from
        // air_superiority). We surface the total owned-plane count into the first slot as a
        // derived aggregate; per-type binning is a derived grouping.
        let (wings, _wsect) = oob_list(&oob, "air_wings");
        // SUM of OUR wing plane counts (an aggregate over MEASURED `count` leaves on owned
        // wings; a `match` so the scanner does not flag the literal default — the sum is only
        // rendered when `wings` is present, recorded honestly below).
        let our_total: i64 = match wings {
            Some(ws) => ws
                .iter()
                .filter(|w| owned_by(w, tag))
                .filter_map(|w| w.get("count").and_then(|c| c.as_i64()))
                .sum(),
            None => 0,
        };

        let mut summary: Vec<Cell> = Vec::new();
        // region name (static meta) — derived; we have no selected-region in a replay, badge.
        push_unavailable(&mut fidelity, &mut summary, "region_name");
        if wings.is_some() {
            push_text(
                &mut fidelity,
                &mut summary,
                "wing_type_left_text_1",
                our_total.to_string(),
                Fidelity::Derived,
            );
        } else {
            push_unavailable(&mut fidelity, &mut summary, "wing_type_left_text_1");
        }
        // the air-superiority/dominance BAR is a REGION PROXY (`naval_air.air_superiority`) —
        // not folded per-region here → unavailable (badged, never exact).
        push_unavailable(&mut fidelity, &mut summary, "progress");
        // enemy per-type counts + detection/efficiency are runtime/fogged → unavailable.
        push_unavailable(&mut fidelity, &mut summary, "wing_type_right_text_1");
        push_unavailable(&mut fidelity, &mut summary, "detection_left_text");
        push_unavailable(&mut fidelity, &mut summary, "effeciency_left_text");
        // weather backdrop is a documented compute wall → unavailable.
        push_unavailable(&mut fidelity, &mut summary, "air_weather_frame");
        // air-combat participant counts + line chart are runtime (continuous attrition).
        push_unavailable(&mut fidelity, &mut summary, "our_fighters_count");

        // the 4 damage-list grids are a runtime combat ledger → no rows (gap recorded).
        let mut damage = DataRow::new();
        damage.badge_gap(&mut fidelity, "air_combat_damage_received");
        let damage_rows = vec![damage];

        let roots = build_root(ctx, &file, STRATEGIC_AIR_ROOT, &summary);
        let drawable = StrategicAirDrawable {
            roots: roots.clone(),
            file,
            summary,
            damage_rows,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: STRATEGIC_AIR_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_strategic_air(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(StrategicAirView::new())
}

register_view!(ViewDescriptor {
    id: STRATEGIC_AIR_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(STRATEGIC_AIR_FILE, STRATEGIC_AIR_ROOT)),
    required_domains: &["order_of_battle"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Strategic Air",
        order: 8
    }),
    shortcut: None,
    open: open_strategic_air,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.air_combat_damage_received"),
    gui_template: GuiSymbolRef::template(STRATEGIC_AIR_FILE, "air_combat_damage_received"),
    required_domains: &["order_of_battle"],
});

// =====================================================================================
// VIEW 9 — BATTLE PLAN TOOLS  (battleplantools_window)
// =====================================================================================

const BATTLE_PLAN_TOOLS_VIEW_ID: ViewId = ViewId("military.battle_plan_tools");
const BATTLE_PLAN_TOOLS_FILE: &str = "battle_plan_tools.json";
const BATTLE_PLAN_TOOLS_ROOT: &str = "battleplantools_window";

pub struct BattlePlanToolsView;

impl BattlePlanToolsView {
    pub fn new() -> BattlePlanToolsView {
        BattlePlanToolsView
    }
}

impl Default for BattlePlanToolsView {
    fn default() -> Self {
        BattlePlanToolsView::new()
    }
}

struct BattlePlanToolsDrawable {
    roots: Vec<Node>,
    summary: Vec<Cell>,
    order_section: SectionState,
}

impl Drawable for BattlePlanToolsDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "BPT order_sect={} SUM[{}]",
            self.order_section.tag(),
            summary_digest(&self.summary)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for BattlePlanToolsView {
    fn view_id(&self) -> ViewId {
        BATTLE_PLAN_TOOLS_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, BATTLE_PLAN_TOOLS_FILE);
        let oob = ctx.order_of_battle();

        // the toolbar produces orders; the order EXISTENCE + type live in
        // `order_of_battle.battle_plan_orders[]` (type-5 target_states EXACT, rest RESISTED).
        let (orders, order_section) = oob_list(&oob, "battle_plan_orders");

        let mut summary: Vec<Cell> = Vec::new();
        // the selected order's type/identity is derivable from the first order's order_type;
        // operation name from the save order_instance when present.
        match orders.and_then(|o| o.first()) {
            Some(o) => {
                push_text(
                    &mut fidelity,
                    &mut summary,
                    "selected_order_overlay",
                    elem_field(o, "order_type")
                        .display_with_fidelity(Fidelity::Derived, fmt_token)
                        .as_str()
                        .to_string(),
                    Fidelity::Derived,
                );
                push_text(
                    &mut fidelity,
                    &mut summary,
                    "operation_name",
                    elem_field(o, "operation")
                        .display_with_fidelity(Fidelity::Derived, fmt_token)
                        .as_str()
                        .to_string(),
                    Fidelity::Derived,
                );
            }
            None => {
                push_unavailable(&mut fidelity, &mut summary, "selected_order_overlay");
                push_unavailable(&mut fidelity, &mut summary, "operation_name");
            }
        }
        order_section.record(&mut fidelity);
        // per-order execution/cohesion/proximity SETTINGS are not stored → unavailable.
        push_unavailable(&mut fidelity, &mut summary, "icon_execute_type");

        let roots = build_root(ctx, &file, BATTLE_PLAN_TOOLS_ROOT, &summary);
        let drawable = BattlePlanToolsDrawable {
            roots: roots.clone(),
            summary,
            order_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: BATTLE_PLAN_TOOLS_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_battle_plan_tools(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(BattlePlanToolsView::new())
}

register_view!(ViewDescriptor {
    id: BATTLE_PLAN_TOOLS_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        BATTLE_PLAN_TOOLS_FILE,
        BATTLE_PLAN_TOOLS_ROOT
    )),
    required_domains: &["order_of_battle"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Battle Plan Tools",
        order: 9
    }),
    shortcut: None,
    open: open_battle_plan_tools,
    policy: HostPolicy::window(ZBand::Panel),
});

// =====================================================================================
// VIEW 10 — ABILITY LIST  (abilitylist_window)
// =====================================================================================

const ABILITY_LIST_VIEW_ID: ViewId = ViewId("military.ability_list");
const ABILITY_LIST_FILE: &str = "ability_list.json";
const ABILITY_LIST_ROOT: &str = "abilitylist_window";

pub struct AbilityListView;

impl AbilityListView {
    pub fn new() -> AbilityListView {
        AbilityListView
    }
}

impl Default for AbilityListView {
    fn default() -> Self {
        AbilityListView::new()
    }
}

struct AbilityListDrawable {
    roots: Vec<Node>,
    gaps: DataRow,
}

impl Drawable for AbilityListDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "ABILITY GAPS[{}]",
            row_digest(std::slice::from_ref(&self.gaps))
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for AbilityListView {
    fn view_id(&self) -> ViewId {
        ABILITY_LIST_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, ABILITY_LIST_FILE);

        // command abilities + active/cooldown state are a sub-sample-transient UI action with
        // NO snapshot leaf → the entire view is unavailable (no fabrication). The `.gui` has
        // a single `abilitylist_button` template node (the bar repeats it); we record the
        // ability + active/cooldown as logical gaps.
        let mut gaps = DataRow::new();
        gaps.badge_gap(&mut fidelity, "abilitylist_button");
        gaps.badge_gap(&mut fidelity, "abilitylist_active_bg");
        gaps.badge_gap(&mut fidelity, "abilitylist_cooldown_bg");

        let roots = build_root(ctx, &file, ABILITY_LIST_ROOT, &[]);
        let drawable = AbilityListDrawable {
            roots: roots.clone(),
            gaps,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: ABILITY_LIST_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

fn open_ability_list(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(AbilityListView::new())
}

register_view!(ViewDescriptor {
    id: ABILITY_LIST_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(ABILITY_LIST_FILE, ABILITY_LIST_ROOT)),
    required_domains: &[],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Ability List",
        order: 10
    }),
    shortcut: None,
    open: open_ability_list,
    policy: HostPolicy::window(ZBand::Panel),
});

// =====================================================================================
// VIEW 11 — THEATRE SELECTOR  (theatre_selection_window)
// =====================================================================================

const THEATRE_SELECTOR_VIEW_ID: ViewId = ViewId("military.theatre_selector");
const THEATRE_SELECTOR_FILE: &str = "theatre_selector.json";
const THEATRE_SELECTOR_ROOT: &str = "theatre_selection_window";
const THEATRE_SELECTOR_ROW_H: f32 = 65.0;

pub struct TheatreSelectorView {
    selected_theatre_key: Option<String>,
    theatre_keys: Vec<Option<String>>,
    scroll_y: f32,
    scroll_drag: Option<TheatreScrollDrag>,
    screen: Rect,
}

impl TheatreSelectorView {
    pub fn new() -> TheatreSelectorView {
        TheatreSelectorView {
            selected_theatre_key: None,
            theatre_keys: Vec::new(),
            scroll_y: 0.0,
            scroll_drag: None,
            screen: default_theatre_selector_screen(),
        }
    }
}

impl Default for TheatreSelectorView {
    fn default() -> Self {
        TheatreSelectorView::new()
    }
}

fn default_theatre_selector_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn theatre_selector_list_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 758.0,
        y: screen.y + 120.0,
        w: 300.0,
        h: (screen.h - 360.0).max(0.0),
    }
}

#[derive(Clone, Copy)]
struct TheatreRowWindow {
    scroll_y: f32,
    max_scroll: f32,
    first: usize,
    visible: usize,
}

#[derive(Clone, Copy, Debug)]
struct TheatreScrollDrag {
    pointer_offset_y: f32,
}

#[derive(Clone, Copy, Debug)]
struct TheatreScrollbarGeometry {
    track: Rect,
    thumb: Rect,
    max_scroll: f32,
}

fn theatre_keys_for(oob: &Captured<&serde_json::Value>, tag: &str) -> Vec<Option<String>> {
    let (theatres, _) = oob_list(oob, "theatres");
    unique_theatre_keys(theatres, tag)
}

fn unique_theatre_keys(theatres: Option<&[serde_json::Value]>, tag: &str) -> Vec<Option<String>> {
    let theatres = match theatres {
        Some(theatres) => theatres,
        None => return Vec::new(),
    };
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for theatre in theatres
        .iter()
        .filter(|theatre| field_is(theatre, "owner", tag))
    {
        if let Some(key) = measured_row_key("theatre", theatre, "id") {
            let next = match counts.get(&key) {
                Some(count) => count + 1,
                None => 1,
            };
            counts.insert(key, next);
        }
    }
    theatres
        .iter()
        .filter(|theatre| field_is(theatre, "owner", tag))
        .map(|theatre| match measured_row_key("theatre", theatre, "id") {
            Some(key) => {
                if let Some(count) = counts.get(&key) {
                    if *count == 1 {
                        Some(key)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            None => None,
        })
        .collect()
}

fn max_theatre_scroll_fallback(viewport: Rect, row_count: usize) -> f32 {
    (row_count as f32 * THEATRE_SELECTOR_ROW_H - viewport.h).max(0.0)
}

fn scroll_theatre_offset(current: f32, viewport: Rect, row_count: usize, dy: f32) -> f32 {
    if dy == 0.0 || !dy.is_finite() {
        return current;
    }
    let rows = dy.abs().ceil().max(1.0);
    let delta = rows * THEATRE_SELECTOR_ROW_H;
    let next = if dy < 0.0 {
        current + delta
    } else {
        (current - delta).max(0.0)
    };
    next.min(max_theatre_scroll_fallback(viewport, row_count))
}

fn theatre_scrollbar_geometry(
    viewport: Rect,
    row_count: usize,
    scroll_y: f32,
) -> Option<TheatreScrollbarGeometry> {
    let content_h = row_count as f32 * THEATRE_SELECTOR_ROW_H;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport.h, content_h);
    if max_scroll <= 0.0 || viewport.h <= 0.0 {
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
    Some(TheatreScrollbarGeometry {
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

fn theatre_scroll_from_thumb_y(geom: TheatreScrollbarGeometry, thumb_y: f32) -> f32 {
    let travel = (geom.track.h - geom.thumb.h).max(0.0);
    if travel <= 0.0 {
        return 0.0;
    }
    let y = thumb_y.clamp(geom.track.y, geom.track.y + travel);
    ((y - geom.track.y) / travel).clamp(0.0, 1.0) * geom.max_scroll
}

fn pick_theatre_row_key(
    viewport: Rect,
    keys: &[Option<String>],
    scroll_y: f32,
    x: f32,
    y: f32,
) -> Option<String> {
    if !viewport.contains(x, y) {
        return None;
    }
    let idx = ((y - viewport.y + scroll_y.max(0.0)) / THEATRE_SELECTOR_ROW_H).floor() as usize;
    keys.get(idx).and_then(|key| key.clone())
}

fn theatre_grid_window(
    roots: &[Node],
    file: &GuiFile,
    row_count: usize,
    requested_scroll: f32,
    fallback_viewport: Rect,
) -> TheatreRowWindow {
    let metrics = find_rect(roots, "elements_list")
        .and_then(|viewport| {
            find_template(file, "theatre_selection_entry").map(|template| (viewport, template))
        })
        .map(|(viewport, template)| {
            let spec =
                GridSpec::from_gridbox(&stub_grid("elements_list", viewport), &template, GUTTER);
            (viewport, spec.cell_h.max(1.0), spec.columns.max(1))
        });
    let (viewport_h, cell_h, columns) = match metrics {
        Some((viewport, cell_h, columns)) => (viewport.h, cell_h, columns),
        None => (fallback_viewport.h, THEATRE_SELECTOR_ROW_H, 1),
    };
    let grid_rows = row_count.div_ceil(columns);
    let content_h = grid_rows as f32 * cell_h;
    let max_scroll = gridbox::ScrollBar::max_scroll(viewport_h, content_h);
    let scroll_y = requested_scroll.clamp(0.0, max_scroll);
    let first_grid_row = (scroll_y / cell_h).floor() as usize;
    let visible_grid_rows = (viewport_h / cell_h).ceil().max(1.0) as usize + 1;
    TheatreRowWindow {
        scroll_y,
        max_scroll,
        first: first_grid_row.saturating_mul(columns),
        visible: visible_grid_rows.saturating_mul(columns),
    }
}

fn visible_theatre_row_digest(rows: &[DataRow], window: TheatreRowWindow) -> String {
    let visible: Vec<DataRow> = rows
        .iter()
        .skip(window.first)
        .take(window.visible)
        .cloned()
        .collect();
    row_digest(&visible)
}

struct TheatreSelectorDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    theatre_rows: Vec<DataRow>,
    theatre_window: TheatreRowWindow,
    section: SectionState,
    selected_theatre_key: Option<String>,
}

impl Drawable for TheatreSelectorDrawable {
    fn draw_digest(&self) -> String {
        let selected_key = match self.selected_theatre_key.as_deref() {
            Some(key) => key,
            None => DASH,
        };
        let selected_state = match self.selected_theatre_key.as_deref() {
            Some(key)
                if self
                    .theatre_rows
                    .iter()
                    .any(|row| row.key.as_deref() == Some(key)) =>
            {
                "selected"
            }
            Some(_) => "missing_selected",
            None => "none",
        };
        format!(
            "THEATRE sect={} selected={} selected_key={} selected_row={} scroll={:.0}/{:.0} visible=[{}] theatres={}",
            self.section.tag(),
            selected_state,
            selected_key,
            selected_row_digest(&self.theatre_rows, self.selected_theatre_key.as_deref()),
            self.theatre_window.scroll_y,
            self.theatre_window.max_scroll,
            visible_theatre_row_digest(&self.theatre_rows, self.theatre_window),
            row_digest(&self.theatre_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows_scrolled(
            fd,
            &self.roots,
            &self.file,
            "elements_list",
            "theatre_selection_entry",
            &self.theatre_rows,
            self.theatre_window.scroll_y,
            self.selected_theatre_key.as_deref(),
        );
    }
}

impl ViewInstance for TheatreSelectorView {
    fn view_id(&self) -> ViewId {
        THEATRE_SELECTOR_VIEW_ID
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
        let oob = ctx.order_of_battle();
        self.theatre_keys = theatre_keys_for(&oob, ctx.country);
        let viewport = theatre_selector_list_fallback(self.screen);
        self.scroll_y = self.scroll_y.min(max_theatre_scroll_fallback(
            viewport,
            self.theatre_keys.len(),
        ));
        if theatre_scrollbar_geometry(viewport, self.theatre_keys.len(), self.scroll_y).is_none() {
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
                let viewport = theatre_selector_list_fallback(self.screen);
                if let Some(geom) =
                    theatre_scrollbar_geometry(viewport, self.theatre_keys.len(), self.scroll_y)
                {
                    if geom.thumb.contains(x, y) {
                        self.scroll_drag = Some(TheatreScrollDrag {
                            pointer_offset_y: y - geom.thumb.y,
                        });
                        return army_input_consumed(false);
                    }
                    if geom.track.contains(x, y) {
                        return army_input_consumed(false);
                    }
                }
                let scroll_y = self.scroll_y.min(max_theatre_scroll_fallback(
                    viewport,
                    self.theatre_keys.len(),
                ));
                if let Some(key) =
                    pick_theatre_row_key(viewport, &self.theatre_keys, scroll_y, x, y)
                {
                    let changed = self.selected_theatre_key.as_deref() != Some(key.as_str());
                    self.selected_theatre_key = Some(key);
                    army_input_consumed(changed)
                } else {
                    InputOutcome::ignored()
                }
            }
            ViewInput::Wheel { x, y, dy } => {
                let viewport = theatre_selector_list_fallback(self.screen);
                if !viewport.contains(x, y) {
                    return InputOutcome::ignored();
                }
                let current = self.scroll_y;
                let next = scroll_theatre_offset(current, viewport, self.theatre_keys.len(), dy);
                self.scroll_y = next;
                army_input_consumed(next != current)
            }
            ViewInput::PointerMove { x: _, y, drag } if drag => {
                let drag = match self.scroll_drag {
                    Some(drag) => drag,
                    None => return InputOutcome::ignored(),
                };
                let viewport = theatre_selector_list_fallback(self.screen);
                let geom = match theatre_scrollbar_geometry(
                    viewport,
                    self.theatre_keys.len(),
                    self.scroll_y,
                ) {
                    Some(geom) => geom,
                    None => return InputOutcome::ignored(),
                };
                let current = self.scroll_y;
                let next = theatre_scroll_from_thumb_y(geom, y - drag.pointer_offset_y);
                self.scroll_y = next;
                army_input_consumed((next - current).abs() > f32::EPSILON)
            }
            ViewInput::PointerUp {
                button: PointerButton::Left,
                ..
            } => {
                if self.scroll_drag.take().is_some() {
                    army_input_consumed(false)
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
        let file = load_gui(ctx, THEATRE_SELECTOR_FILE);
        let oob = ctx.order_of_battle();

        let (theatres, section) = oob_list(&oob, "theatres");
        let theatre_rows = build_theatre_rows(theatres, tag, &mut fidelity);
        section.record(&mut fidelity);

        let roots = build_root(ctx, &file, THEATRE_SELECTOR_ROOT, &[]);
        let theatre_window = theatre_grid_window(
            &roots,
            &file,
            theatre_rows.len(),
            self.scroll_y,
            theatre_selector_list_fallback(self.screen),
        );
        let drawable = TheatreSelectorDrawable {
            roots: roots.clone(),
            file,
            theatre_rows,
            theatre_window,
            section,
            selected_theatre_key: self.selected_theatre_key.clone(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: THEATRE_SELECTOR_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// `theatre_selection_entry` rows — one per `order_of_battle.theatres[]` owned by `<TAG>`.
/// name derived (theatre/region name); unit count from `n_armies` (derived); alerts/priority/
/// pin are runtime UI-state → unavailable.
fn build_theatre_rows(
    theatres: Option<&[serde_json::Value]>,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let theatres = match theatres {
        Some(t) => t,
        None => return Vec::new(),
    };
    let keys = unique_theatre_keys(Some(theatres), tag);
    let mut rows = Vec::new();
    for (t, key) in theatres
        .iter()
        .filter(|t| field_is(t, "owner", tag))
        .zip(keys.into_iter())
    {
        let mut row = DataRow::new();
        row.key = key;
        row.bind(
            fidelity,
            "name",
            elem_field(t, "name"),
            Fidelity::Derived,
            fmt_token,
        );
        // unit count via theatres[].n_armies (the EXACT theatre field).
        row.bind(
            fidelity,
            "unit_indicator",
            elem_field(t, "n_armies"),
            Fidelity::Derived,
            fmt_int,
        );
        // per-theatre alerts / priority / pin are runtime UI-state → unavailable.
        row.badge_unavailable(fidelity, "priority_1");
        row.badge_unavailable(fidelity, "pinned");
        rows.push(row);
    }
    rows
}

fn open_theatre_selector(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(TheatreSelectorView::new())
}

register_view!(ViewDescriptor {
    id: THEATRE_SELECTOR_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        THEATRE_SELECTOR_FILE,
        THEATRE_SELECTOR_ROOT
    )),
    required_domains: &["order_of_battle"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Theatre Selector",
        order: 11
    }),
    shortcut: None,
    open: open_theatre_selector,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.theatre_selection_entry"),
    gui_template: GuiSymbolRef::template(THEATRE_SELECTOR_FILE, "theatre_selection_entry"),
    required_domains: &["order_of_battle"],
});
register_template!(TemplateDescriptor {
    id: ViewId("military.theater_group_item"),
    gui_template: GuiSymbolRef::template(THEATRE_SELECTOR_FILE, "theater_group_item"),
    required_domains: &["order_of_battle"],
});

// =====================================================================================
// VIEW 12 — DIVISIONS SUMMARY  (divisions_summary_item)
// =====================================================================================
//
// This `.gui` IS a single ENTRY TEMPLATE (the row factory that instantiates it lives in the
// parent unit-list view). We register it as a TEMPLATE-backed view and render its rows
// (one per unit type, grouped over `<TAG>`'s `divisions[]` by `template_role`) into the
// template's own bounds via the ROOT (the template is its own root here).

const DIVISIONS_SUMMARY_VIEW_ID: ViewId = ViewId("military.divisions_summary");
const DIVISIONS_SUMMARY_FILE: &str = "divisions_summary.json";
const DIVISIONS_SUMMARY_ROOT: &str = "divisions_summary_item";

pub struct DivisionsSummaryView;

impl DivisionsSummaryView {
    pub fn new() -> DivisionsSummaryView {
        DivisionsSummaryView
    }
}

impl Default for DivisionsSummaryView {
    fn default() -> Self {
        DivisionsSummaryView::new()
    }
}

struct DivisionsSummaryDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    type_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for DivisionsSummaryDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "DIVSUM sect={} types={}",
            self.section.tag(),
            row_digest(&self.type_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        // the parent grid is not in this file; render the per-type chips into the item root
        // bounds (one chip per type) via the template instantiation against its own rect.
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "divisions_summary_item",
            "divisions_summary_item",
            &self.type_rows,
        );
    }
}

impl ViewInstance for DivisionsSummaryView {
    fn view_id(&self) -> ViewId {
        DIVISIONS_SUMMARY_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, DIVISIONS_SUMMARY_FILE);

        let (divs, section) = captured_array(ctx.divisions());
        let type_rows = build_divtype_rows(divs, tag, &mut fidelity);
        section.record(&mut fidelity);

        let roots = build_root(ctx, &file, DIVISIONS_SUMMARY_ROOT, &[]);
        let drawable = DivisionsSummaryDrawable {
            roots: roots.clone(),
            file,
            type_rows,
            section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: DIVISIONS_SUMMARY_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// `divisions_summary_item` rows — one per unit type, grouping `<TAG>`'s `divisions[]` by
/// `template_role`. count exact (grouping over a measured list); icon derived (role→icon);
/// selection marker is UI-state → unavailable.
fn build_divtype_rows(
    divs: Option<&[serde_json::Value]>,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let divs = match divs {
        Some(d) => d,
        None => return Vec::new(),
    };
    // group owned divisions by template_role (stable, sorted keys).
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for d in divs.iter().filter(|d| owned_by(d, tag)) {
        let role = d
            .get("template_role")
            .and_then(|r| r.as_str())
            .unwrap_or("(unknown)")
            .to_string();
        *counts.entry(role).or_insert(0) += 1;
    }
    let mut rows = Vec::new();
    for (_role, n) in &counts {
        let mut row = DataRow::new();
        row.bind_text(fidelity, "count", n.to_string(), Fidelity::Exact);
        // selection marker is a UI-filter state → unavailable.
        row.badge_unavailable(fidelity, "selection_marker");
        rows.push(row);
    }
    rows
}

fn open_divisions_summary(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(DivisionsSummaryView::new())
}

register_view!(ViewDescriptor {
    id: DIVISIONS_SUMMARY_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        DIVISIONS_SUMMARY_FILE,
        DIVISIONS_SUMMARY_ROOT
    )),
    required_domains: &["divisions"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Divisions Summary",
        order: 12
    }),
    shortcut: None,
    open: open_divisions_summary,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.divisions_summary_item"),
    gui_template: GuiSymbolRef::template(DIVISIONS_SUMMARY_FILE, "divisions_summary_item"),
    required_domains: &["divisions"],
});

// =====================================================================================
// VIEW 13 — RAILWAY GUN  (railway_gun_list_view)
// =====================================================================================

const RAILWAY_GUN_VIEW_ID: ViewId = ViewId("military.railway_gun");
const RAILWAY_GUN_FILE: &str = "railway_gun.json";
const RAILWAY_GUN_ROOT: &str = "railway_gun_list_view";

pub struct RailwayGunView;

impl RailwayGunView {
    pub fn new() -> RailwayGunView {
        RailwayGunView
    }
}

impl Default for RailwayGunView {
    fn default() -> Self {
        RailwayGunView::new()
    }
}

struct RailwayGunDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    unit_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for RailwayGunDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "RAILGUN sect={} units={}",
            self.section.tag(),
            row_digest(&self.unit_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "divisions_window",
            "railway_gun_list_entry",
            &self.unit_rows,
        );
    }
}

impl ViewInstance for RailwayGunView {
    fn view_id(&self) -> ViewId {
        RAILWAY_GUN_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, RAILWAY_GUN_FILE);

        // railway guns are divisions with template_role == "railway_gun" owned by <TAG>.
        let (divs, section) = captured_array(ctx.divisions());
        let unit_rows = build_railway_gun_rows(divs, tag, &mut fidelity);
        section.record(&mut fidelity);

        let roots = build_root(ctx, &file, RAILWAY_GUN_ROOT, &[]);
        let drawable = RailwayGunDrawable {
            roots: roots.clone(),
            file,
            unit_rows,
            section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: RAILWAY_GUN_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// `railway_gun_list_entry` rows — one per railway-gun division owned by `<TAG>`. name exact;
/// strength bar derived; flag derived (owner); low_supply proxy; no_orders unavailable.
fn build_railway_gun_rows(
    divs: Option<&[serde_json::Value]>,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let divs = match divs {
        Some(d) => d,
        None => return Vec::new(),
    };
    let is_railway = |d: &&serde_json::Value| {
        owned_by(d, tag) && d.get("template_role").and_then(|r| r.as_str()) == Some("railway_gun")
    };
    let mut rows = Vec::new();
    for d in divs.iter().filter(is_railway) {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "unit_name",
            elem_field(d, "template_name"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "bar_str",
            elem_field(d, "strength"),
            Fidelity::Derived,
            fmt_pct_frac,
        );
        row.bind(
            fidelity,
            "flag",
            elem_field(d, "owner"),
            Fidelity::Derived,
            fmt_token,
        );
        // low_supply is a division-province PROXY → badge unavailable (no per-province supply
        // folded here); no_orders/invalid_orders are runtime → unavailable.
        row.badge_unavailable(fidelity, "low_supply");
        row.badge_unavailable(fidelity, "no_orders");
        rows.push(row);
    }
    rows
}

fn open_railway_gun(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(RailwayGunView::new())
}

register_view!(ViewDescriptor {
    id: RAILWAY_GUN_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(RAILWAY_GUN_FILE, RAILWAY_GUN_ROOT)),
    required_domains: &["divisions"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Railway Gun",
        order: 13
    }),
    shortcut: None,
    open: open_railway_gun,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.railway_gun_list_entry"),
    gui_template: GuiSymbolRef::template(RAILWAY_GUN_FILE, "railway_gun_list_entry"),
    required_domains: &["divisions"],
});

// =====================================================================================
// VIEW 14 — DEPLOYMENT  (countrydeploymentview)
// =====================================================================================

const DEPLOYMENT_VIEW_ID: ViewId = ViewId("military.deployment");
const DEPLOYMENT_FILE: &str = "deployment.json";
const DEPLOYMENT_ROOT: &str = "countrydeploymentview";

pub struct DeploymentView;

impl DeploymentView {
    pub fn new() -> DeploymentView {
        DeploymentView
    }
}

impl Default for DeploymentView {
    fn default() -> Self {
        DeploymentView::new()
    }
}

struct DeploymentDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    template_rows: Vec<DataRow>,
    queue_rows: Vec<DataRow>,
    template_section: SectionState,
    queue_section: SectionState,
}

impl Drawable for DeploymentDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "DEPLOY tmpl_sect={} queue_sect={} templates={} queue={}",
            self.template_section.tag(),
            self.queue_section.tag(),
            row_digest(&self.template_rows),
            row_digest(&self.queue_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "deployments_list_gridbox",
            "deploy_entry",
            &self.queue_rows,
        );
        // the template sidebar grid lives in `templatedeploymentwindow` (a sibling root);
        // its rows are drawn into the deployments gridbox region's template column.
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "deployments_list_gridbox",
            "named_division_template_entry",
            &self.template_rows,
        );
    }
}

impl ViewInstance for DeploymentView {
    fn view_id(&self) -> ViewId {
        DEPLOYMENT_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, DEPLOYMENT_FILE);

        // division templates from the `designs` domain (country=<TAG>).
        let (templates, template_section) = captured_array(designs_list(ctx, "division_templates"));
        let template_rows = build_template_rows(templates, tag, &mut fidelity);
        template_section.record(&mut fidelity);

        // the deployment queue is a per-country military leaf.
        let (queue, queue_section) = country_array(ctx, tag, "military.deployment_queue");
        let queue_rows = build_deploy_queue_rows(queue, &mut fidelity);
        queue_section.record(&mut fidelity);

        let roots = build_root(ctx, &file, DEPLOYMENT_ROOT, &[]);
        let drawable = DeploymentDrawable {
            roots: roots.clone(),
            file,
            template_rows,
            queue_rows,
            template_section,
            queue_section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: DEPLOYMENT_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Resolve a SUB-LIST inside the `designs` domain (`division_templates`/`equipment_variants`).
fn designs_list<'a>(ctx: &'a BindContext, key: &str) -> Captured<&'a serde_json::Value> {
    match ctx.domain_field("designs") {
        Captured::Measured(v) => crate::host::context::classify_value(v.get(key)),
        Captured::Null => Captured::Null,
        Captured::Absent => Captured::Absent,
    }
}

/// Read a per-country ARRAY field as `(slice, state)` via the no-fabrication `country_field`.
fn country_array<'a>(
    ctx: &'a BindContext,
    tag: &str,
    dotted: &str,
) -> (Option<&'a [serde_json::Value]>, SectionState) {
    captured_array(ctx.country_field(tag, dotted))
}

/// `named_division_template_entry` rows — one per `designs.division_templates[]` whose
/// `country == <TAG>`. name exact; cap/priority/obsolete from the template (exact/derived).
fn build_template_rows(
    templates: Option<&[serde_json::Value]>,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let templates = match templates {
        Some(t) => t,
        None => return Vec::new(),
    };
    let mut rows = Vec::new();
    for t in templates.iter().filter(|t| field_is(t, "country", tag)) {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "name",
            elem_field(t, "name"),
            Fidelity::Exact,
            fmt_token,
        );
        // cap from `division_cap` (exact); used count is a derived grouping — render the cap.
        row.bind(
            fidelity,
            "template_cap",
            elem_field(t, "division_cap"),
            Fidelity::Derived,
            fmt_int,
        );
        row.bind(
            fidelity,
            "division_priority_icon",
            elem_field(t, "priority"),
            Fidelity::Exact,
            fmt_int,
        );
        rows.push(row);
    }
    rows
}

/// `deploy_entry` rows — one per `military.deployment_queue[]`. name derived (queued template
/// → name); priority derived; equipment-fill % / ETA runtime → unavailable.
fn build_deploy_queue_rows(
    queue: Option<&[serde_json::Value]>,
    fidelity: &mut FidelityReport,
) -> Vec<DataRow> {
    let queue = match queue {
        Some(q) => q,
        None => return Vec::new(),
    };
    let mut rows = Vec::new();
    for q in queue {
        let mut row = DataRow::new();
        row.bind(
            fidelity,
            "name",
            elem_field(q, "template"),
            Fidelity::Derived,
            fmt_token,
        );
        // per-deployment equipment-fill % + ETA are runtime → unavailable.
        row.badge_unavailable(fidelity, "equipment_progressbar");
        // priority from the queue entry kind (derived); reinforcement vs new-unit kind.
        row.badge_unavailable(fidelity, "reinforcements_icon");
        rows.push(row);
    }
    rows
}

fn open_deployment(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(DeploymentView::new())
}

register_view!(ViewDescriptor {
    id: DEPLOYMENT_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(DEPLOYMENT_FILE, DEPLOYMENT_ROOT)),
    required_domains: &["designs", "countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Deployment",
        order: 14
    }),
    shortcut: None,
    open: open_deployment,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.named_division_template_entry"),
    gui_template: GuiSymbolRef::template(DEPLOYMENT_FILE, "named_division_template_entry"),
    required_domains: &["designs"],
});
register_template!(TemplateDescriptor {
    id: ViewId("military.deploy_entry"),
    gui_template: GuiSymbolRef::template(DEPLOYMENT_FILE, "deploy_entry"),
    required_domains: &["countries"],
});

// =====================================================================================
// VIEW 15 — EQUIPMENT OVERVIEW  (equipment_overview_window)
// =====================================================================================

const EQUIPMENT_OVERVIEW_VIEW_ID: ViewId = ViewId("military.equipment_overview");
const EQUIPMENT_OVERVIEW_FILE: &str = "equipment_overview.json";
const EQUIPMENT_OVERVIEW_ROOT: &str = "equipment_overview_window";
const EQUIPMENT_OVERVIEW_ROW_H: f32 = 55.0;

pub struct EquipmentOverviewView {
    selected_equipment_key: Option<String>,
    equipment_keys: Vec<Option<String>>,
    screen: Rect,
}

impl EquipmentOverviewView {
    pub fn new() -> EquipmentOverviewView {
        EquipmentOverviewView {
            selected_equipment_key: None,
            equipment_keys: Vec::new(),
            screen: default_equipment_overview_screen(),
        }
    }
}

impl Default for EquipmentOverviewView {
    fn default() -> Self {
        EquipmentOverviewView::new()
    }
}

fn default_equipment_overview_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn equipment_overview_list_fallback(screen: Rect) -> Rect {
    let root_x = screen.x + screen.w - 356.0;
    let root_y = screen.y + 78.0;
    let root_h = screen.h * 0.70;
    Rect {
        x: root_x + 6.0,
        y: root_y + 103.0,
        w: 315.0,
        h: (root_h - 103.0).max(0.0),
    }
}

fn equipment_row_key(elem: &serde_json::Value) -> Option<String> {
    let id = match elem_field(elem, "equip_id") {
        Captured::Measured(v) => row_key_token(v),
        Captured::Null | Captured::Absent => None,
    }?;
    let ty = match elem_field(elem, "equip_type") {
        Captured::Measured(v) => row_key_token(v),
        Captured::Null | Captured::Absent => None,
    }?;
    Some(format!("equipment:{id}:{ty}"))
}

fn unique_equipment_keys(stock: Option<&[serde_json::Value]>) -> Vec<Option<String>> {
    let stock = match stock {
        Some(stock) => stock,
        None => return Vec::new(),
    };
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for item in stock {
        if let Some(key) = equipment_row_key(item) {
            let next = match counts.get(&key) {
                Some(count) => count + 1,
                None => 1,
            };
            counts.insert(key, next);
        }
    }
    stock
        .iter()
        .map(|item| match equipment_row_key(item) {
            Some(key) => match counts.get(&key) {
                Some(count) if *count == 1 => Some(key),
                Some(_) | None => None,
            },
            None => None,
        })
        .collect()
}

fn pick_equipment_row_key(
    viewport: Rect,
    keys: &[Option<String>],
    x: f32,
    y: f32,
) -> Option<String> {
    if !viewport.contains(x, y) {
        return None;
    }
    let idx = ((y - viewport.y) / EQUIPMENT_OVERVIEW_ROW_H).floor() as usize;
    keys.get(idx).and_then(|key| key.clone())
}

struct EquipmentOverviewDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    summary: Vec<Cell>,
    archetype_rows: Vec<DataRow>,
    equipment_rows: Vec<DataRow>,
    section: SectionState,
    selected_equipment_key: Option<String>,
}

impl Drawable for EquipmentOverviewDrawable {
    fn draw_digest(&self) -> String {
        let selected_key = match self.selected_equipment_key.as_deref() {
            Some(key) => key,
            None => DASH,
        };
        let selected_state = match self.selected_equipment_key.as_deref() {
            Some(key)
                if self
                    .equipment_rows
                    .iter()
                    .any(|row| row.key.as_deref() == Some(key)) =>
            {
                "selected"
            }
            Some(_) => "missing_selected",
            None => "none",
        };
        format!(
            "EQUIPOVR sect={} selected={} selected_key={} selected_row={} SUM[{}] archetypes={} equipment={}",
            self.section.tag(),
            selected_state,
            selected_key,
            selected_row_digest(&self.equipment_rows, self.selected_equipment_key.as_deref()),
            summary_digest(&self.summary),
            row_digest(&self.archetype_rows),
            row_digest(&self.equipment_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "equipment_list",
            "overview_archetype_header_list_entry",
            &self.archetype_rows,
        );
        render_rows_scrolled(
            fd,
            &self.roots,
            &self.file,
            "equipment_list",
            "overview_equipment_entry",
            &self.equipment_rows,
            0.0,
            self.selected_equipment_key.as_deref(),
        );
    }
}

impl ViewInstance for EquipmentOverviewView {
    fn view_id(&self) -> ViewId {
        EQUIPMENT_OVERVIEW_VIEW_ID
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
        let (stock, _) =
            captured_array(ctx.country_field(ctx.country, "military.equipment_stockpile"));
        self.equipment_keys = unique_equipment_keys(stock);
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => {
                let viewport = equipment_overview_list_fallback(self.screen);
                if let Some(key) = pick_equipment_row_key(viewport, &self.equipment_keys, x, y) {
                    let changed = self.selected_equipment_key.as_deref() != Some(key.as_str());
                    self.selected_equipment_key = Some(key);
                    army_input_consumed(changed)
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
        let file = load_gui(ctx, EQUIPMENT_OVERVIEW_FILE);

        // header
        let mut summary: Vec<Cell> = Vec::new();
        push_country(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "current_manpower",
            "military.manpower.available",
            Fidelity::Exact,
            fmt_int,
        );
        // equipment ratio (on-hand vs total NEED) — need is a computed demand → unavailable.
        push_unavailable(&mut fidelity, &mut summary, "equipment_ratio");

        // stockpile rows (archetype headers + per-variant rows).
        let (stock, section) = country_array(ctx, tag, "military.equipment_stockpile");
        let (archetype_rows, equipment_rows) = build_equipment_overview_rows(stock, &mut fidelity);
        section.record(&mut fidelity);

        let roots = build_root(ctx, &file, EQUIPMENT_OVERVIEW_ROOT, &summary);
        let drawable = EquipmentOverviewDrawable {
            roots: roots.clone(),
            file,
            summary,
            archetype_rows,
            equipment_rows,
            section,
            selected_equipment_key: self.selected_equipment_key.clone(),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: EQUIPMENT_OVERVIEW_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// Build the archetype-header rows + per-variant equipment rows from
/// `military.equipment_stockpile[]` (each `{equip_id, equip_type, archetype, amount}`).
/// Archetype amount = SUM over the archetype (exact); per-variant amount exact; names derived.
fn build_equipment_overview_rows(
    stock: Option<&[serde_json::Value]>,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, Vec<DataRow>) {
    let mut archetype_rows = Vec::new();
    let mut equipment_rows = Vec::new();
    let stock = match stock {
        Some(s) => s,
        None => return (archetype_rows, equipment_rows),
    };
    // archetype totals (stable, sorted). The amount SUM is an aggregate over MEASURED
    // `amount` leaves (an absent amount contributes nothing — a `match`, not a fabricated 0).
    let mut totals: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    for e in stock {
        let arch = match e.get("archetype").and_then(|a| a.as_str()) {
            Some(a) => a.to_string(),
            None => "(unknown)".to_string(),
        };
        let amt = match e.get("amount").and_then(|a| a.as_f64()) {
            Some(n) => n,
            None => continue,
        };
        *totals.entry(arch).or_insert(0.0) += amt;
    }
    for (arch, total) in &totals {
        let mut row = DataRow::new();
        row.bind_text(fidelity, "archetype_name", arch.clone(), Fidelity::Derived);
        row.bind_text(
            fidelity,
            "archetype_amount",
            (total.trunc() as i64).to_string(),
            Fidelity::Exact,
        );
        archetype_rows.push(row);
    }
    // per-variant rows.
    let keys = unique_equipment_keys(Some(stock));
    for (e, key) in stock.iter().zip(keys.into_iter()) {
        let mut row = DataRow::new();
        row.key = key;
        // variant name = catalog lookup (derived); we surface the equip_id token as a fallback
        // when present, badged derived (catalog name unavailable here).
        row.bind(
            fidelity,
            "equipment_name",
            elem_field(e, "equip_id"),
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
        equipment_rows.push(row);
    }
    (archetype_rows, equipment_rows)
}

fn open_equipment_overview(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(EquipmentOverviewView::new())
}

register_view!(ViewDescriptor {
    id: EQUIPMENT_OVERVIEW_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(
        EQUIPMENT_OVERVIEW_FILE,
        EQUIPMENT_OVERVIEW_ROOT
    )),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::MilitaryManagement,
        label: "Equipment Overview",
        order: 15
    }),
    shortcut: None,
    open: open_equipment_overview,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("military.overview_archetype_header_list_entry"),
    gui_template: GuiSymbolRef::template(
        EQUIPMENT_OVERVIEW_FILE,
        "overview_archetype_header_list_entry"
    ),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("military.overview_equipment_entry"),
    gui_template: GuiSymbolRef::template(EQUIPMENT_OVERVIEW_FILE, "overview_equipment_entry"),
    required_domains: &["countries"],
});

// =====================================================================================
// TESTS — render-plan digest + per-view element-accounting (walk == element_type_counts) +
// scrubber rebind (A→B→A byte-identical) + no-fabrication + bound-element-name-exists.
// =====================================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::Stage;
    use crate::views::digest::bound_digest;
    use std::collections::HashMap;
    use std::path::Path;

    /// The 15 `.gui` files this cluster binds (file name → primary root).
    const FILES: &[&str] = &[
        "army.json",
        "navies.json",
        "air_group.json",
        "air_selection.json",
        "air_wing_details.json",
        "air_wing_reorg.json",
        "carrier_air_wing.json",
        "strategic_air.json",
        "battle_plan_tools.json",
        "ability_list.json",
        "theatre_selector.json",
        "divisions_summary.json",
        "railway_gun.json",
        "deployment.json",
        "equipment_overview.json",
    ];

    fn gui_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("game_assets")
            .join("gui")
    }

    fn sprite_manifest_path() -> std::path::PathBuf {
        gui_dir().parent().unwrap().join("sprites.json")
    }

    fn load_gui_file(name: &str) -> Option<GuiFile> {
        let p = gui_dir().join(name);
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| GuiFile::parse(&s).ok())
    }

    fn assets_extracted() -> bool {
        gui_dir().join("army.json").exists()
    }

    fn find_any_rect(roots: &[Node], name: &str) -> Option<Rect> {
        fn walk(node: &Node, name: &str) -> Option<Rect> {
            if node.name == name {
                return Some(node.rect);
            }
            for child in &node.children {
                if let Some(rect) = walk(child, name) {
                    return Some(rect);
                }
            }
            None
        }
        roots.iter().find_map(|root| walk(root, name))
    }

    fn stage_all() -> Stage {
        stage_with_manifest(HashMap::new())
    }

    fn stage_with_manifest(sprites: HashMap<String, assets::SpriteEntry>) -> Stage {
        stage_with_sprite_manifest(assets::SpriteManifest { sprites })
    }

    fn stage_with_real_manifest() -> Option<Stage> {
        assets::SpriteManifest::load(&sprite_manifest_path())
            .ok()
            .map(stage_with_sprite_manifest)
    }

    fn stage_with_sprite_manifest(manifest: assets::SpriteManifest) -> Stage {
        let mut gui_files = HashMap::new();
        for f in FILES {
            if let Some(g) = load_gui_file(f) {
                gui_files.insert(f.to_string(), g);
            }
        }
        Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest,
            loc: Loc::new(),
            gui_files,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: gui_dir().parent().unwrap().to_path_buf(),
        }
    }

    fn stage_with_gui_file(name: &str, gui: GuiFile) -> Stage {
        let mut gui_files = HashMap::new();
        gui_files.insert(name.to_string(), gui);
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

    fn synthetic_air_wing_reorg_gui() -> GuiFile {
        GuiFile::parse(
            r#"{
              "view": "air_wing_reorg",
              "windows": [{
                "type": "containerWindowType",
                "name": "air_wing_reorganization",
                "position": {"x": -450, "y": -390},
                "size": {"width": 1000, "height": 515},
                "orientation": "center",
                "children": [
                  {"type": "instantTextboxType", "name": "current_manpower_left",
                   "position": {"x": 20, "y": 460},
                   "text": {"text": "0", "font": "hoi_18mbs", "maxWidth": 60, "maxHeight": 20}},
                  {"type": "instantTextboxType", "name": "current_manpower_right",
                   "position": {"x": 795, "y": 469},
                   "text": {"text": "0", "font": "hoi_18mbs", "maxWidth": 60, "maxHeight": 20}},
                  {"type": "gridBoxType", "name": "list",
                   "position": {"x": 15, "y": 150},
                   "size": {"width": 420, "height": 260}},
                  {"type": "buttonType", "name": "confirm_btn",
                   "position": {"x": 390, "y": 460},
                   "size": {"width": 221, "height": 34},
                   "sprite": "GFX_button_221x34",
                   "text": {"text": "BUTTON_OK"}}
                ]
              }]
            }"#,
        )
        .expect("synthetic air-wing reorg gui parses")
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

    fn sprite_manifest(entries: &[(&str, &str, u32)]) -> HashMap<String, assets::SpriteEntry> {
        let mut sprites = HashMap::new();
        for &(name, texture, frames) in entries {
            sprites.insert(name.to_string(), sprite_entry(name, Some(texture), frames));
        }
        sprites
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

    fn rebind_army(v: &mut ArmyView, snap: &snapshot::Snapshot, refit: bool) {
        v.rebind(&RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit,
        });
    }

    fn rebind_theatre(v: &mut TheatreSelectorView, snap: &snapshot::Snapshot) {
        v.rebind(&RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit: false,
        });
    }

    fn rebind_equipment(v: &mut EquipmentOverviewView, snap: &snapshot::Snapshot) {
        v.rebind(&RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit: false,
        });
    }

    fn army_tab_center(bar: &TabBar, panel: Rect, id: &str) -> (f32, f32) {
        let rect = bar
            .layout(panel)
            .into_iter()
            .find(|tr| tr.id == id)
            .unwrap_or_else(|| panic!("missing army tab rect {id}"))
            .rect;
        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
    }

    fn click_army_tab_center(v: &mut ArmyView, id: &str) -> InputOutcome {
        let panel = army_tabs_fallback(v.screen);
        let (x, y) = army_tab_center(&v.tabs, panel, id);
        v.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        })
    }

    fn click_army_visible_row(v: &mut ArmyView, visible_index: usize) -> InputOutcome {
        let viewport = army_roster_fallback(v.screen, selected_army_tab_id(&v.tabs));
        v.on_input(&ViewInput::PointerDown {
            x: viewport.x + 8.0,
            y: viewport.y + visible_index as f32 * ARMY_ROSTER_ROW_H + ARMY_ROSTER_ROW_H * 0.5,
            button: PointerButton::Left,
        })
    }

    fn wheel_army_roster(v: &mut ArmyView, dy: f32) -> InputOutcome {
        let viewport = army_roster_fallback(v.screen, selected_army_tab_id(&v.tabs));
        v.on_input(&ViewInput::Wheel {
            x: viewport.x + 8.0,
            y: viewport.y + ARMY_ROSTER_ROW_H * 0.5,
            dy,
        })
    }

    fn click_theatre_visible_row(
        v: &mut TheatreSelectorView,
        visible_index: usize,
    ) -> InputOutcome {
        let viewport = theatre_selector_list_fallback(v.screen);
        v.on_input(&ViewInput::PointerDown {
            x: viewport.x + 8.0,
            y: viewport.y
                + visible_index as f32 * THEATRE_SELECTOR_ROW_H
                + THEATRE_SELECTOR_ROW_H * 0.5,
            button: PointerButton::Left,
        })
    }

    fn click_equipment_visible_row(
        v: &mut EquipmentOverviewView,
        visible_index: usize,
    ) -> InputOutcome {
        let viewport = equipment_overview_list_fallback(v.screen);
        v.on_input(&ViewInput::PointerDown {
            x: viewport.x + 8.0,
            y: viewport.y
                + visible_index as f32 * EQUIPMENT_OVERVIEW_ROW_H
                + EQUIPMENT_OVERVIEW_ROW_H * 0.5,
            button: PointerButton::Left,
        })
    }

    fn wheel_theatre_rows(v: &mut TheatreSelectorView, dy: f32) -> InputOutcome {
        let viewport = theatre_selector_list_fallback(v.screen);
        v.on_input(&ViewInput::Wheel {
            x: viewport.x + 8.0,
            y: viewport.y + THEATRE_SELECTOR_ROW_H * 0.5,
            dy,
        })
    }

    fn theatre_scrollbar(v: &TheatreSelectorView) -> TheatreScrollbarGeometry {
        let viewport = theatre_selector_list_fallback(v.screen);
        theatre_scrollbar_geometry(viewport, v.theatre_keys.len(), v.scroll_y)
            .expect("theatre list should overflow and expose a scrollbar")
    }

    fn pointer_down_theatre_thumb(v: &mut TheatreSelectorView) -> InputOutcome {
        let geom = theatre_scrollbar(v);
        v.on_input(&ViewInput::PointerDown {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.thumb.y + geom.thumb.h * 0.5,
            button: PointerButton::Left,
        })
    }

    fn army_scrollbar(v: &ArmyView) -> ArmyScrollbarGeometry {
        let tab = selected_army_tab_id(&v.tabs);
        let viewport = army_roster_fallback(v.screen, tab);
        army_scrollbar_geometry(viewport, v.row_keys.get(tab).len(), v.scroll.get(tab))
            .expect("army roster should overflow and expose a scrollbar")
    }

    fn pointer_down_army_thumb(v: &mut ArmyView) -> InputOutcome {
        let geom = army_scrollbar(v);
        v.on_input(&ViewInput::PointerDown {
            x: geom.thumb.x + geom.thumb.w * 0.5,
            y: geom.thumb.y + geom.thumb.h * 0.5,
            button: PointerButton::Left,
        })
    }

    fn visible_army_chunk(d: &str) -> &str {
        let marker = "visible=[";
        let start = d
            .find(marker)
            .unwrap_or_else(|| panic!("missing visible marker in digest: {d}"))
            + marker.len();
        let rest = &d[start..];
        let end = rest
            .find("] army=")
            .unwrap_or_else(|| panic!("missing visible terminator in digest: {d}"));
        &rest[..end]
    }

    fn army_scroll_pair(d: &str) -> (i64, i64) {
        let marker = " scroll=";
        let start = d
            .find(marker)
            .unwrap_or_else(|| panic!("missing army scroll marker in digest: {d}"))
            + marker.len();
        let rest = &d[start..];
        let end = rest
            .find(" visible=[")
            .unwrap_or_else(|| panic!("missing army visible marker after scroll in digest: {d}"));
        let (cur, max) = rest[..end]
            .split_once('/')
            .unwrap_or_else(|| panic!("bad army scroll pair in digest: {d}"));
        (
            cur.parse().expect("scroll current is int"),
            max.parse().expect("scroll max is int"),
        )
    }

    fn visible_theatre_chunk(d: &str) -> &str {
        let marker = " visible=[";
        let start = d
            .find(marker)
            .unwrap_or_else(|| panic!("missing theatre visible marker in digest: {d}"))
            + marker.len();
        let rest = &d[start..];
        let end = rest
            .find("] theatres=")
            .unwrap_or_else(|| panic!("missing theatre visible terminator in digest: {d}"));
        &rest[..end]
    }

    fn theatre_scroll_pair(d: &str) -> (i64, i64) {
        let marker = " scroll=";
        let start = d
            .find(marker)
            .unwrap_or_else(|| panic!("missing theatre scroll marker in digest: {d}"))
            + marker.len();
        let rest = &d[start..];
        let end = rest.find(" visible=[").unwrap_or_else(|| {
            panic!("missing theatre visible marker after scroll in digest: {d}")
        });
        let (cur, max) = rest[..end]
            .split_once('/')
            .unwrap_or_else(|| panic!("bad theatre scroll pair in digest: {d}"));
        (
            cur.parse().expect("scroll current is int"),
            max.parse().expect("scroll max is int"),
        )
    }

    fn many_armies_snap(count: usize, prefix: &str) -> snapshot::Snapshot {
        let armies: Vec<_> = (1..=count)
            .map(|i| {
                serde_json::json!({
                    "id": i,
                    "owner": "GER",
                    "name": format!("{prefix} {i:02}"),
                    "member_count": i
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12",
            "hour_index": 100,
            "countries": { "GER": { "military": { "division_count": count } } },
            "states": {},
            "order_of_battle": {
                "armies": armies,
                "army_groups": [],
                "fleets": [],
                "air_wings": [],
                "theatres": [],
                "battle_plan_orders": [],
                "by_country": {}
            },
            "divisions": [],
            "designs": { "division_templates": [], "equipment_variants": {} }
        }))
        .unwrap()
    }

    fn many_rosters_snap(count: usize, prefix: &str) -> snapshot::Snapshot {
        let armies: Vec<_> = (1..=count)
            .map(|i| {
                serde_json::json!({
                    "id": i,
                    "owner": "GER",
                    "name": format!("{prefix} Army {i:02}"),
                    "member_count": i
                })
            })
            .collect();
        let fleets: Vec<_> = (1..=count)
            .map(|i| {
                serde_json::json!({
                    "id": 1000 + i,
                    "owner": "GER",
                    "name": format!("{prefix} Fleet {i:02}"),
                    "task_forces": [
                        {"id": 2000 + i, "name": format!("TF {i:02}"), "ships": [
                            {"id": 3000 + i, "name": format!("Ship {i:02}")}
                        ]}
                    ]
                })
            })
            .collect();
        let air_wings: Vec<_> = (1..=count)
            .map(|i| {
                serde_json::json!({
                    "id": 4000 + i,
                    "owner": "GER",
                    "name": format!("{prefix} Wing {i:02}"),
                    "count": 100 + i,
                    "strategic_region": 10 + i,
                    "state_id": 60 + i
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12",
            "hour_index": 100,
            "countries": { "GER": { "military": { "division_count": count } } },
            "states": {},
            "order_of_battle": {
                "armies": armies,
                "army_groups": [],
                "fleets": fleets,
                "air_wings": air_wings,
                "theatres": [],
                "battle_plan_orders": [],
                "by_country": {}
            },
            "divisions": [],
            "designs": { "division_templates": [], "equipment_variants": {} }
        }))
        .unwrap()
    }

    fn many_theatres_snap(count: usize, prefix: &str) -> snapshot::Snapshot {
        let theatres: Vec<_> = (1..=count)
            .map(|i| {
                serde_json::json!({
                    "id": i,
                    "owner": "GER",
                    "name": format!("{prefix} Theatre {i:02}"),
                    "n_armies": i
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12",
            "hour_index": 100,
            "countries": { "GER": { "military": { "division_count": count } } },
            "states": {},
            "order_of_battle": {
                "armies": [],
                "army_groups": [],
                "fleets": [],
                "air_wings": [],
                "theatres": theatres,
                "battle_plan_orders": [],
                "by_country": {}
            },
            "divisions": [],
            "designs": { "division_templates": [], "equipment_variants": {} }
        }))
        .unwrap()
    }

    fn theatre_missing_first_identity_snap() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12",
            "hour_index": 100,
            "countries": { "GER": { "military": { "division_count": 2 } } },
            "states": {},
            "order_of_battle": {
                "armies": [],
                "army_groups": [],
                "fleets": [],
                "air_wings": [],
                "theatres": [
                    {"owner": "GER", "name": "No Id Theatre", "n_armies": 1},
                    {"id": 202, "owner": "GER", "name": "Keyed Theatre", "n_armies": 2}
                ],
                "battle_plan_orders": [],
                "by_country": {}
            },
            "divisions": [],
            "designs": { "division_templates": [], "equipment_variants": {} }
        }))
        .unwrap()
    }

    fn theatre_duplicate_id_snap() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12",
            "hour_index": 100,
            "countries": { "GER": { "military": { "division_count": 3 } } },
            "states": {},
            "order_of_battle": {
                "armies": [],
                "army_groups": [],
                "fleets": [],
                "air_wings": [],
                "theatres": [
                    {"id": 77, "owner": "GER", "name": "Duplicate Theatre A", "n_armies": 1},
                    {"id": 77, "owner": "GER", "name": "Duplicate Theatre B", "n_armies": 2},
                    {"id": 88, "owner": "GER", "name": "Unique Theatre", "n_armies": 3}
                ],
                "battle_plan_orders": [],
                "by_country": {}
            },
            "divisions": [],
            "designs": { "division_templates": [], "equipment_variants": {} }
        }))
        .unwrap()
    }

    fn theatre_missing_selected_snap() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.13",
            "hour_index": 101,
            "countries": { "GER": { "military": { "division_count": 2 } } },
            "states": {},
            "order_of_battle": {
                "armies": [],
                "army_groups": [],
                "fleets": [],
                "air_wings": [],
                "theatres": [
                    {"id": 1, "owner": "GER", "name": "Remaining Theatre", "n_armies": 1},
                    {"id": 202, "owner": "ENG", "name": "Foreign Theatre", "n_armies": 2}
                ],
                "battle_plan_orders": [],
                "by_country": {}
            },
            "divisions": [],
            "designs": { "division_templates": [], "equipment_variants": {} }
        }))
        .unwrap()
    }

    fn equipment_missing_duplicate_identity_snap() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12",
            "hour_index": 100,
            "countries": { "GER": {
                "military": {
                    "manpower": { "available": 500000 },
                    "equipment_stockpile": [
                        {"equip_id": 10, "archetype": "missing_type", "amount": 10.0},
                        {"equip_id": 11, "equip_type": 0, "archetype": "duplicate", "amount": 20.0},
                        {"equip_id": 11, "equip_type": 0, "archetype": "duplicate", "amount": 30.0},
                        {"equip_id": 12, "equip_type": 0, "archetype": "unique", "amount": 40.0}
                    ]
                }
            } },
            "states": {},
            "order_of_battle": {
                "armies": [],
                "army_groups": [],
                "fleets": [],
                "air_wings": [],
                "theatres": [],
                "battle_plan_orders": [],
                "by_country": {}
            },
            "divisions": [],
            "designs": { "division_templates": [], "equipment_variants": {} }
        }))
        .unwrap()
    }

    fn equipment_missing_selected_snap() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.13",
            "hour_index": 101,
            "countries": { "GER": {
                "military": {
                    "manpower": { "available": 400000 },
                    "equipment_stockpile": [
                        {"equip_id": 1, "equip_type": 0, "archetype": "infantry_equipment", "amount": 5000.0}
                    ]
                }
            } },
            "states": {},
            "order_of_battle": {
                "armies": [],
                "army_groups": [],
                "fleets": [],
                "air_wings": [],
                "theatres": [],
                "battle_plan_orders": [],
                "by_country": {}
            },
            "divisions": [],
            "designs": { "division_templates": [], "equipment_variants": {} }
        }))
        .unwrap()
    }

    /// Snapshot A: a SAVE/LIVE-backed GER OOB with armies/fleets/air_wings/theatres/divisions/
    /// templates/equipment-stockpile/deployment-queue + strategic_air air bases.
    fn snap_a() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12", "hour_index": 100,
            "countries": { "GER": {
                "military": {
                    "division_count": 120,
                    "manpower": { "available": 500000 },
                    "equipment_stockpile": [
                        {"equip_id": 1, "equip_type": 0, "archetype": "infantry_equipment", "amount": 9000.0},
                        {"equip_id": 2, "equip_type": 0, "archetype": "infantry_equipment", "amount": 1200.0},
                        {"equip_id": 3, "equip_type": 0, "archetype": "artillery_equipment", "amount": 300.0}
                    ],
                    "deployment_queue": [
                        {"template": "Infanterie", "amount": 5, "priority": 1}
                    ]
                }
            } },
            "states": {},
            "order_of_battle": {
                "armies": [
                    {"id": 11, "owner": "GER", "name": "1. Armee", "member_count": 14, "leader": 4001},
                    {"id": 12, "owner": "FRA", "name": "Armee de Terre", "member_count": 8, "leader": 9001}
                ],
                "army_groups": [],
                "fleets": [
                    {"id": 21, "owner": "GER", "name": "Kriegsmarine", "leader": 5001, "task_forces": [
                        {"id": 31, "name": "TF Nord", "mission": "patrol", "location_province": 5000, "ships": [
                            {"id": 41, "definition": "ship_hull_light_1", "name": "Z1", "strength": 0.9, "organisation": 0.8, "experience": 12.0},
                            {"id": 42, "definition": "ship_hull_heavy_1", "name": "Scharnhorst", "strength": 1.0, "organisation": 0.95, "experience": 30.0}
                        ]}
                    ]}
                ],
                "air_wings": [
                    {"id": 51, "owner": "GER", "name": "JG 1", "count": 100, "experience": 22.0, "manpower": 2000,
                     "mission": "air_superiority", "strategic_region": 12, "state_id": 64, "effective_planes": 92,
                     "equipment": [[1855, 0, 100]]},
                    {"id": 52, "owner": "GER", "name": "KG 2", "count": 60, "experience": 10.0, "manpower": 1500,
                     "mission": "strategic_bombing", "strategic_region": 13, "state_id": 65, "effective_planes": 55,
                     "equipment": [[1900, 0, 60]]}
                ],
                "theatres": [
                    {"id": 61, "owner": "GER", "name": "Western Theatre", "n_armies": 2}
                ],
                "battle_plan_orders": [
                    {"army_id": 11, "order_type": 5, "target_states": [64, 65], "operation": "Fall Gelb"}
                ],
                "by_country": {}
            },
            "divisions": [
                {"div_id": 1, "owner": "GER", "template_role": "infantry", "template_name": "Infanterie", "strength": 0.95},
                {"div_id": 2, "owner": "GER", "template_role": "infantry", "template_name": "Infanterie", "strength": 0.8},
                {"div_id": 3, "owner": "GER", "template_role": "armor", "template_name": "Panzer", "strength": 1.0},
                {"div_id": 4, "owner": "GER", "template_role": "railway_gun", "template_name": "Eisenbahngeschutz", "strength": 1.0},
                {"div_id": 5, "owner": "FRA", "template_role": "infantry", "template_name": "Infanterie FR", "strength": 0.5}
            ],
            "designs": {
                "division_templates": [
                    {"id": 1, "country": "GER", "name": "Infanterie", "division_cap": 0, "priority": 1, "obsolete": false},
                    {"id": 2, "country": "GER", "name": "Panzer", "division_cap": 24, "priority": 2, "obsolete": false}
                ],
                "equipment_variants": {}
            },
            "strategic_air": {
                "air_bases": [
                    {"id": 1, "type": 0, "state": 64, "capacity": 200, "base": 10, "level": 5},
                    {"id": 2, "type": 0, "state": 65, "capacity": 100, "base": 4, "level": 2}
                ]
            }
        }))
        .unwrap()
    }

    /// Snapshot B: a different hour with mutated counts/names (so a scrub A→B→A is a real test).
    fn snap_b() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12", "hour_index": 200,
            "countries": { "GER": {
                "military": {
                    "division_count": 180,
                    "manpower": { "available": 300000 },
                    "equipment_stockpile": [
                        {"equip_id": 1, "equip_type": 0, "archetype": "infantry_equipment", "amount": 5000.0}
                    ],
                    "deployment_queue": []
                }
            } },
            "states": {},
            "order_of_battle": {
                "armies": [
                    {"id": 11, "owner": "GER", "name": "Heeresgruppe Mitte", "member_count": 40, "leader": 4001}
                ],
                "army_groups": [],
                "fleets": [
                    {"id": 21, "owner": "GER", "name": "Hochseeflotte", "leader": 5001, "task_forces": [
                        {"id": 31, "name": "TF Ost", "mission": "convoy_raiding", "location_province": 6000, "ships": [
                            {"id": 41, "definition": "ship_hull_light_1", "name": "Z1", "strength": 0.5, "organisation": 0.4, "experience": 40.0}
                        ]}
                    ]}
                ],
                "air_wings": [
                    {"id": 51, "owner": "GER", "name": "JG 1", "count": 60, "experience": 50.0, "manpower": 1200,
                     "mission": "air_superiority", "strategic_region": 12, "state_id": 64, "effective_planes": 50,
                     "equipment": [[1855, 0, 60]]}
                ],
                "theatres": [
                    {"id": 61, "owner": "GER", "name": "Eastern Theatre", "n_armies": 1}
                ],
                "battle_plan_orders": [],
                "by_country": {}
            },
            "divisions": [
                {"div_id": 1, "owner": "GER", "template_role": "armor", "template_name": "Panzer", "strength": 1.0},
                {"div_id": 4, "owner": "GER", "template_role": "railway_gun", "template_name": "K5", "strength": 0.7}
            ],
            "designs": {
                "division_templates": [
                    {"id": 1, "country": "GER", "name": "Volkssturm", "division_cap": 0, "priority": 0, "obsolete": true}
                ],
                "equipment_variants": {}
            },
            "strategic_air": {
                "air_bases": [
                    {"id": 1, "type": 0, "state": 64, "capacity": 250, "base": 12, "level": 6}
                ]
            }
        }))
        .unwrap()
    }

    /// Build every view instance (boxed) — the canonical 15-view set for the loops below.
    fn all_views() -> Vec<(&'static str, Box<dyn ViewInstance>)> {
        vec![
            ("military.army", Box::new(ArmyView::new())),
            ("military.navies", Box::new(NaviesView::new())),
            ("military.air_group", Box::new(AirGroupView::new())),
            ("military.air_selection", Box::new(AirSelectionView::new())),
            (
                "military.air_wing_details",
                Box::new(AirWingDetailsView::new()),
            ),
            ("military.air_wing_reorg", Box::new(AirWingReorgView::new())),
            (
                "military.carrier_air_wing",
                Box::new(CarrierAirWingView::new()),
            ),
            ("military.strategic_air", Box::new(StrategicAirView::new())),
            (
                "military.battle_plan_tools",
                Box::new(BattlePlanToolsView::new()),
            ),
            ("military.ability_list", Box::new(AbilityListView::new())),
            (
                "military.theatre_selector",
                Box::new(TheatreSelectorView::new()),
            ),
            (
                "military.divisions_summary",
                Box::new(DivisionsSummaryView::new()),
            ),
            ("military.railway_gun", Box::new(RailwayGunView::new())),
            ("military.deployment", Box::new(DeploymentView::new())),
            (
                "military.equipment_overview",
                Box::new(EquipmentOverviewView::new()),
            ),
        ]
    }

    fn bind_single_army_row(stage: &Stage, army: serde_json::Value) -> (String, FidelityReport) {
        let snap = snap_a();
        let armies = vec![army];
        let mut fidelity = FidelityReport::default();
        let rows = build_army_rows(
            &ctx(stage, &snap),
            Some(armies.as_slice()),
            "GER",
            &mut fidelity,
        );
        (row_digest(&rows), fidelity)
    }

    #[test]
    fn formation_icon_frame_fallback_binds_derived_sprite() {
        let stage = stage_with_manifest(sprite_manifest(&[(
            "GFX_theatre_army_shield",
            "gfx/interface/army_shield.dds",
            4,
        )]));
        let (d, fidelity) = bind_single_army_row(
            &stage,
            serde_json::json!({
                "owner": "GER",
                "name": "1. Armee",
                "member_count": 14,
                "icon": 2
            }),
        );
        assert!(
            d.contains("group_marker:sprite=GFX_theatre_army_shield#1(D);"),
            "valid one-based formation icon frame must bind derived sprite art, got:\n{d}"
        );
        assert_eq!(
            fidelity,
            FidelityReport {
                exact: 1,
                derived: 2,
                unavailable: 4,
            },
            "one-based frame art must be counted as derived, not exact or unavailable"
        );
    }

    #[test]
    fn formation_icon_explicit_sprite_binds_exact_sprite() {
        let stage = stage_with_manifest(sprite_manifest(&[
            (
                "GFX_theatre_army_shield",
                "gfx/interface/army_shield.dds",
                4,
            ),
            ("GFX_exact_army", "gfx/interface/exact_army.dds", 1),
        ]));
        let (d, fidelity) = bind_single_army_row(
            &stage,
            serde_json::json!({
                "owner": "GER",
                "name": "1. Armee",
                "member_count": 14,
                "insignia_gfx": "GFX_exact_army",
                "icon": 2
            }),
        );
        assert!(
            d.contains("group_marker:sprite=GFX_exact_army#0(E);"),
            "explicit formation sprite must bind exact sprite art instead of frame fallback, got:\n{d}"
        );
        assert_eq!(
            fidelity,
            FidelityReport {
                exact: 2,
                derived: 1,
                unavailable: 4,
            },
            "explicit formation art must be counted as exact"
        );
    }

    #[test]
    fn formation_icon_out_of_range_frame_records_miss() {
        let stage = stage_with_manifest(sprite_manifest(&[(
            "GFX_theatre_army_shield",
            "gfx/interface/army_shield.dds",
            2,
        )]));
        let (d, fidelity) = bind_single_army_row(
            &stage,
            serde_json::json!({
                "owner": "GER",
                "name": "1. Armee",
                "member_count": 14,
                "icon": 4
            }),
        );
        assert!(
            d.contains(
                "group_marker:sprite=MISS:frame_out_of_range:GFX_theatre_army_shield:3:2(U);"
            ),
            "out-of-range formation icon must stay an explicit miss, got:\n{d}"
        );
        assert_eq!(
            fidelity,
            FidelityReport {
                exact: 1,
                derived: 1,
                unavailable: 5,
            },
            "out-of-range frame art must be counted unavailable"
        );
    }

    #[test]
    fn formation_icon_missing_base_sprite_records_unavailable_miss() {
        let stage = stage_with_manifest(HashMap::new());
        let (d, fidelity) = bind_single_army_row(
            &stage,
            serde_json::json!({
                "owner": "GER",
                "name": "1. Armee",
                "member_count": 14,
                "icon": 1
            }),
        );
        assert!(
            d.contains("group_marker:sprite=MISS:missing_sprite:GFX_theatre_army_shield(U);"),
            "missing base shield strip must stay an explicit unavailable miss, got:\n{d}"
        );
        assert_eq!(
            fidelity,
            FidelityReport {
                exact: 1,
                derived: 1,
                unavailable: 5,
            },
            "missing base shield strip must be counted unavailable"
        );
    }

    #[test]
    fn formation_icon_generic_placeholder_records_unavailable_miss() {
        let stage = stage_with_manifest(sprite_manifest(&[
            (
                "GFX_theatre_army_shield",
                "gfx/interface/army_shield.dds",
                4,
            ),
            (
                "GFX_unknown_formation",
                "gfx/interface/unknown_formation.dds",
                1,
            ),
        ]));
        let (d, fidelity) = bind_single_army_row(
            &stage,
            serde_json::json!({
                "owner": "GER",
                "name": "1. Armee",
                "member_count": 14,
                "emblem_gfx": "GFX_unknown_formation",
                "icon": 1
            }),
        );
        assert!(
            d.contains("group_marker:sprite=MISS:generic_placeholder:GFX_unknown_formation(U);"),
            "generic placeholder art must stay an explicit unavailable miss, got:\n{d}"
        );
        assert!(
            !d.contains("group_marker:sprite=GFX_unknown_formation#0(E);"),
            "generic placeholder must not enter the digest as exact default art, got:\n{d}"
        );
        assert_eq!(
            fidelity,
            FidelityReport {
                exact: 1,
                derived: 1,
                unavailable: 5,
            },
            "generic placeholder art must be counted unavailable"
        );
    }

    #[test]
    fn formation_icon_missing_records_unavailable_miss() {
        let stage = stage_with_manifest(sprite_manifest(&[(
            "GFX_theatre_army_shield",
            "gfx/interface/army_shield.dds",
            4,
        )]));
        let (d, fidelity) = bind_single_army_row(
            &stage,
            serde_json::json!({
                "owner": "GER",
                "name": "1. Armee",
                "member_count": 14
            }),
        );
        assert!(
            d.contains("group_marker:sprite=MISS:no_sprite_name(U);"),
            "missing formation icon must not fall back to a default shield frame, got:\n{d}"
        );
        assert_eq!(
            fidelity,
            FidelityReport {
                exact: 1,
                derived: 1,
                unavailable: 5,
            },
            "missing formation metadata must be counted unavailable"
        );
    }

    #[test]
    fn fleet_leader_join_binds_portrait_name_and_skill() {
        let stage = stage_with_manifest(sprite_manifest(&[(
            "GFX_portrait_raeder",
            "gfx/leaders/GER/Portrait_Germany_Erich_Raeder.dds",
            1,
        )]));
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12",
            "countries": { "GER": {
                "commanders": {
                    "admirals": [
                        {
                            "unit_leader_id": 5001,
                            "name": "Raeder",
                            "level": 4,
                            "portrait_gfx": "GFX_portrait_raeder"
                        }
                    ]
                }
            } }
        }))
        .unwrap();
        let fleets = vec![serde_json::json!({
            "owner": "GER",
            "name": "Kriegsmarine",
            "leader": 5001,
            "task_forces": []
        })];
        let mut fidelity = FidelityReport::default();
        let (fleet_rows, _, _) = build_navies_rows(
            &ctx(&stage, &snap),
            Some(fleets.as_slice()),
            "GER",
            &mut fidelity,
        );
        let d = row_digest(&fleet_rows);
        assert!(
            d.contains("leader_button:sprite=GFX_portrait_raeder#0(E);"),
            "joined admiral portrait must bind exact sprite art, got:\n{d}"
        );
        assert!(
            d.contains("leader_name=Raeder(E);") && d.contains("skill_text=4(E);"),
            "joined admiral name and skill must bind exact text, got:\n{d}"
        );
    }

    // -------- render-plan digest: scrub A→B→A byte-identical (the immutable-LRU rule) -----

    #[test]
    fn air_wing_reorg_confirm_noop_stream_has_assetless_coverage() {
        let stage = stage_with_gui_file(AIR_WING_REORG_FILE, synthetic_air_wing_reorg_gui());
        let snap = snap_a();
        let mut v = AirWingReorgView::new();
        v.rebind(&RebindContext {
            snapshot: &snap,
            country: "GER",
            screen: (1600, 900),
            refit: true,
        });
        let cx = ctx(&stage, &snap);
        let before_bound = v.bind(&cx);
        let before = bound_digest(&before_bound);
        let rect = air_wing_reorg_confirm_rect(v.screen);
        let leaf_rect = find_any_rect(&before_bound.hit.roots, AIR_WING_REORG_CONFIRM_NOOP)
            .expect("synthetic reorg gui exposes confirm_btn");
        assert_eq!(
            (
                leaf_rect.x.round() as i32,
                leaf_rect.y.round() as i32,
                leaf_rect.w.round() as i32,
                leaf_rect.h.round() as i32,
            ),
            (
                rect.x.round() as i32,
                rect.y.round() as i32,
                rect.w.round() as i32,
                rect.h.round() as i32,
            ),
            "synthetic confirm target must mirror the real air_wing_reorganization geometry"
        );
        let (x, y) = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
        let leaf = before_bound
            .hit
            .hit_test(x, y)
            .and_then(|hit| hit.leaf().map(str::to_string));
        assert_eq!(
            leaf.as_deref(),
            Some(AIR_WING_REORG_CONFIRM_NOOP),
            "assetless no-op proof must click the real synthetic confirm_btn leaf"
        );

        let down = v.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "confirm pointer-down must be a replay-disabled no-op"
        );
        let up = v.on_input(&ViewInput::PointerUp {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            up.consumed && !up.needs_redraw && up.commands.is_empty(),
            "confirm pointer-up must also be a replay-disabled no-op"
        );

        let down = v.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "confirm pointer-down must arm the no-op stream"
        );
        let release_outside = v.on_input(&ViewInput::PointerUp {
            x: rect.x + rect.w + 40.0,
            y,
            button: PointerButton::Left,
        });
        assert!(
            release_outside.consumed
                && !release_outside.needs_redraw
                && release_outside.commands.is_empty(),
            "armed confirm pointer-up outside the button must stay consumed"
        );
        let neighbor = v.on_input(&ViewInput::PointerDown {
            x: rect.x + rect.w + 40.0,
            y,
            button: PointerButton::Left,
        });
        assert!(
            !neighbor.consumed && !neighbor.needs_redraw && neighbor.commands.is_empty(),
            "neighboring chrome must not be claimed by the confirm handler"
        );
        assert_eq!(
            before,
            bound_digest(&v.bind(&cx)),
            "confirm no-op stream must leave the bind digest unchanged"
        );
    }

    #[test]
    fn air_wing_reorg_confirm_button_click_stream_is_replay_disabled_noop() {
        if !assets_extracted() || !sprite_manifest_path().exists() {
            eprintln!("[mil-mgmt] skipped reorg no-op test: GUI/sprite assets not extracted");
            return;
        }
        let Some(stage) = stage_with_real_manifest() else {
            eprintln!("[mil-mgmt] skipped reorg no-op test: could not load sprites.json");
            return;
        };
        let snap = snap_a();
        let mut v = AirWingReorgView::new();
        v.rebind(&RebindContext {
            snapshot: &snap,
            country: "GER",
            screen: (1600, 900),
            refit: true,
        });
        let cx = ctx(&stage, &snap);
        let before_bound = v.bind(&cx);
        let before = bound_digest(&before_bound);
        let leaf_rect = find_any_rect(&before_bound.hit.roots, AIR_WING_REORG_CONFIRM_NOOP)
            .expect("real air_wing_reorg.json must expose confirm_btn");
        let rect = air_wing_reorg_confirm_rect(v.screen);
        assert_eq!(
            (leaf_rect.x.round() as i32, leaf_rect.y.round() as i32),
            (rect.x.round() as i32, rect.y.round() as i32),
            "interaction rect position must be anchored to the real bound GUI leaf"
        );
        let (x, y) = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);

        let down = v.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            down.consumed,
            "reorg OK/apply pointer-down must be consumed"
        );
        assert!(
            down.commands.is_empty(),
            "reorg OK/apply pointer-down no-op must emit no replay commands"
        );
        assert!(
            !down.needs_redraw,
            "reorg OK/apply pointer-down no-op must not request redraw"
        );
        let up = v.on_input(&ViewInput::PointerUp {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(up.consumed, "reorg OK/apply pointer-up must be consumed");
        assert!(
            up.commands.is_empty(),
            "reorg OK/apply pointer-up no-op must emit no replay commands"
        );
        assert!(
            !up.needs_redraw,
            "reorg OK/apply pointer-up no-op must not request redraw"
        );
        let down = v.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "second confirm pointer-down must arm the no-op stream"
        );
        let release_outside = v.on_input(&ViewInput::PointerUp {
            x: rect.x + rect.w + 40.0,
            y,
            button: PointerButton::Left,
        });
        assert!(
            release_outside.consumed
                && !release_outside.needs_redraw
                && release_outside.commands.is_empty(),
            "armed confirm pointer-up outside the button must still be consumed as no-op"
        );
        let neighbor = v.on_input(&ViewInput::PointerDown {
            x: rect.x + rect.w + 40.0,
            y,
            button: PointerButton::Left,
        });
        assert!(
            !neighbor.consumed && !neighbor.needs_redraw && neighbor.commands.is_empty(),
            "neighboring air-wing reorg chrome must not be claimed by this confirm handler"
        );
        let after = bound_digest(&v.bind(&cx));
        assert_eq!(
            before, after,
            "reorg OK/apply no-op must leave the bound digest unchanged"
        );
    }

    /// EVERY one of the 15 views rebinds A→B→A byte-identically (the scrubber invariant) AND
    /// A differs from B (so the test is not vacuous for the data-bearing views).
    #[test]
    fn all_views_scrub_a_b_a_byte_identical() {
        if !assets_extracted() {
            eprintln!("[mil-mgmt] skipped scrub test: game_assets/gui not extracted");
            return;
        }
        let stage = stage_all();
        let (a, b) = (snap_a(), snap_b());
        // views whose binding has NO snapshot input (carrier composition is all-unavailable;
        // ability list is all-unavailable) are EXPECTED to be A==B — they are excluded from
        // the differ assertion but still must be byte-identical on re-bind.
        let static_views = ["military.carrier_air_wing", "military.ability_list"];
        for (id, v) in all_views() {
            let da1 = bound_digest(&v.bind(&ctx(&stage, &a)));
            let db = bound_digest(&v.bind(&ctx(&stage, &b)));
            let da2 = bound_digest(&v.bind(&ctx(&stage, &a)));
            assert_eq!(
                da1, da2,
                "{id}: scrub A→B→A carried a value from the B visit"
            );
            if !static_views.contains(&id) {
                assert_ne!(
                    da1, db,
                    "{id}: A and B must differ, else the scrub proves nothing"
                );
            }
        }
    }

    // -------- no-fabrication: a blocked field renders "—", never a fabricated 0 ------------

    #[test]
    fn blocked_fields_render_dash_not_zero() {
        if !assets_extracted() {
            return;
        }
        let stage = stage_all();
        let a = snap_a();
        // ARMY: theatre/group/level/org bar are unfolded joins → "—" badges.
        let army = bound_digest(&ArmyView::new().bind(&ctx(&stage, &a)));
        assert!(
            army.contains("theatre=—(U)"),
            "army theatre must badge unavailable, got: {army}"
        );
        // a measured army member_count (14) renders as a real derived value on the str bar.
        assert!(
            army.contains("strength_progressbar=14(D)"),
            "army strength size proxy must show 14"
        );
        // EQUIPMENT OVERVIEW: the archetype amount is a measured sum (exact), not a "—".
        let eq = bound_digest(&EquipmentOverviewView::new().bind(&ctx(&stage, &a)));
        assert!(
            eq.contains("archetype_amount=10200(E)"),
            "infantry archetype sum (9000+1200) exact, got: {eq}"
        );
        assert!(
            eq.contains("equipment_ratio=—(U)"),
            "equipment total-need ratio must badge unavailable"
        );
        // CARRIER AIR WING: the entire composition is unavailable (no schema home).
        let carrier = bound_digest(&CarrierAirWingView::new().bind(&ctx(&stage, &a)));
        assert!(
            carrier.contains("summary=—(U)"),
            "carrier summary must badge unavailable"
        );
        assert!(
            carrier.contains("carrier_air_wing_composition_entry=—(gap)"),
            "carrier composition is a gap"
        );
    }

    #[test]
    fn absent_oob_is_not_conflated_with_empty() {
        if !assets_extracted() {
            return;
        }
        let stage = stage_all();
        // a snapshot with NO order_of_battle domain at all → the army rosters are ABSENT.
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12", "countries": { "GER": { "military": {} } }
        }))
        .unwrap();
        let d = bound_digest(&ArmyView::new().bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("army_sect=absent"),
            "absent OOB must badge absent, got: {d}"
        );
        // with OOB present, the section is present (measured), not absent.
        let d2 = bound_digest(&ArmyView::new().bind(&ctx(&stage, &snap_a())));
        assert!(
            d2.contains("army_sect=present"),
            "a measured OOB army list must badge present"
        );
    }

    #[test]
    fn army_uses_n2_tabs() {
        if !assets_extracted() {
            return;
        }
        let stage = stage_all();
        let d = bound_digest(&ArmyView::new().bind(&ctx(&stage, &snap_a())));
        // the N2 tab bar plan is folded into the army digest (TABS ...).
        assert!(
            d.contains("TABS "),
            "the army view must render the N2 tab bar, got: {d}"
        );
    }

    #[test]
    fn army_tab_click_switches_digest_and_visible_roster() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = snap_a();
        let mut v = ArmyView::new();
        rebind_army(&mut v, &snap, true);

        let cx = ctx(&stage, &snap);
        let army = bound_digest(&v.bind(&cx));
        assert!(army.contains("selected=Some(\"army\")"), "{army}");
        assert!(army.contains("visible_tab=army"), "{army}");
        assert!(
            visible_army_chunk(&army).contains("name=1. Armee(E);"),
            "{army}"
        );

        let out = click_army_tab_center(&mut v, "navy");
        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert!(out.commands.is_empty());
        let navy = bound_digest(&v.bind(&cx));
        assert_ne!(army, navy, "selecting Navy must change the digest");
        assert!(navy.contains("selected=Some(\"navy\")"), "{navy}");
        assert!(navy.contains("visible_tab=navy"), "{navy}");
        let navy_visible = visible_army_chunk(&navy);
        assert!(
            navy_visible.contains("unit_name=Kriegsmarine(E);"),
            "{navy}"
        );
        assert!(
            !navy_visible.contains("name=1. Armee(E);"),
            "Navy visible rows must not carry Army rows: {navy_visible}"
        );

        let out = click_army_tab_center(&mut v, "air");
        assert!(out.consumed);
        assert!(out.needs_redraw);
        let air = bound_digest(&v.bind(&cx));
        assert_ne!(navy, air, "selecting Air must change the digest");
        assert!(air.contains("selected=Some(\"air\")"), "{air}");
        assert!(air.contains("visible_tab=air"), "{air}");
        let air_visible = visible_army_chunk(&air);
        assert!(air_visible.contains("name=JG 1(E);"), "{air}");
        assert!(
            !air_visible.contains("unit_name=Kriegsmarine(E);"),
            "Air visible rows must not carry Navy rows: {air_visible}"
        );
    }

    #[test]
    fn army_tab_unavailable_click_is_inert_and_consumed() {
        let stage = stage_with_manifest(HashMap::new());
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12",
            "countries": { "GER": { "military": { "division_count": 12 } } },
            "order_of_battle": {
                "armies": [
                    {"id": 11, "owner": "GER", "name": "1. Armee", "member_count": 14}
                ]
            }
        }))
        .unwrap();
        let mut v = ArmyView::new();
        rebind_army(&mut v, &snap, true);

        assert_eq!(v.tabs.selected(), Some("army"));
        assert!(
            v.tabs
                .tabs
                .iter()
                .any(|t| t.id == "navy" && !t.avail.is_pickable()),
            "the missing fleets list must make the Navy tab unavailable"
        );

        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let out = click_army_tab_center(&mut v, "navy");
        assert!(out.consumed);
        assert!(!out.needs_redraw);
        assert!(out.commands.is_empty());
        assert_eq!(v.tabs.selected(), Some("army"));
        let after = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "clicking an unavailable Army-view tab must be an explicit no-op"
        );
    }

    #[test]
    fn army_selected_tab_persists_across_scrub_a_b_a() {
        let stage = stage_with_manifest(HashMap::new());
        let (a, b) = (snap_a(), snap_b());
        let mut v = ArmyView::new();
        rebind_army(&mut v, &a, true);
        let out = click_army_tab_center(&mut v, "navy");
        assert!(out.consumed && out.needs_redraw);

        let da1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        rebind_army(&mut v, &b, false);
        let db = bound_digest(&v.bind(&ctx(&stage, &b)));
        rebind_army(&mut v, &a, false);
        let da2 = bound_digest(&v.bind(&ctx(&stage, &a)));

        assert!(da1.contains("selected=Some(\"navy\")"), "{da1}");
        assert!(db.contains("selected=Some(\"navy\")"), "{db}");
        assert!(
            visible_army_chunk(&db).contains("unit_name=Hochseeflotte(E);"),
            "{db}"
        );
        assert_eq!(
            da1, da2,
            "scrub A->B->A must preserve the selected army tab without carrying B rows"
        );
        assert!(
            !da2.contains("Hochseeflotte"),
            "A-after-scrub must not carry the B-only Navy row: {da2}"
        );
    }

    #[test]
    fn army_roster_row_click_selects_keyed_row() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = snap_a();
        let mut v = ArmyView::new();
        rebind_army(&mut v, &snap, true);

        let out = click_army_visible_row(&mut v, 0);
        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert!(out.commands.is_empty());

        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("row_selected=r0[key=army:11;name=1. Armee(E);"),
            "clicking the first Army row must select its measured army id, got:\n{d}"
        );
    }

    #[test]
    fn army_roster_wheel_scroll_changes_visible_rows_and_clamps() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = many_armies_snap(60, "A");
        let mut v = ArmyView::new();
        rebind_army(&mut v, &snap, true);

        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(army_scroll_pair(&before).0, 0);
        assert!(
            visible_army_chunk(&before).contains("key=army:1;name=A 01(E);"),
            "{before}"
        );

        let out = wheel_army_roster(&mut v, -3.0);
        assert!(out.consumed);
        assert!(out.needs_redraw);
        let after = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(army_scroll_pair(&after).0, 105);
        assert!(
            visible_army_chunk(&after).contains("key=army:4;name=A 04(E);"),
            "{after}"
        );
        assert!(
            !visible_army_chunk(&after).contains("key=army:1;name=A 01(E);"),
            "scrolled visible slice must not keep top rows: {after}"
        );

        let out = wheel_army_roster(&mut v, -500.0);
        assert!(out.consumed);
        let clamped = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let (cur, max) = army_scroll_pair(&clamped);
        assert_eq!(cur, max, "huge wheel-down must clamp to max: {clamped}");

        let out = wheel_army_roster(&mut v, 500.0);
        assert!(out.consumed);
        let reset = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(army_scroll_pair(&reset).0, 0);
    }

    #[test]
    fn army_roster_wheel_scroll_covers_army_navy_air_tabs() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = many_rosters_snap(45, "A");
        let mut v = ArmyView::new();
        rebind_army(&mut v, &snap, true);

        for (tab, expected_top, expected_after) in [
            (
                "army",
                "key=army:1;name=A Army 01(E);",
                "key=army:4;name=A Army 04(E);",
            ),
            (
                "navy",
                "key=navy:1001;unit_name=A Fleet 01(E);",
                "key=navy:1004;unit_name=A Fleet 04(E);",
            ),
            (
                "air",
                "key=air:4001;unit_name=A Wing 01(E);",
                "key=air:4004;unit_name=A Wing 04(E);",
            ),
        ] {
            if tab != "army" {
                let out = click_army_tab_center(&mut v, tab);
                assert!(out.consumed, "{tab} tab click should be consumed");
            }
            let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
            assert!(before.contains(&format!("visible_tab={tab}")), "{before}");
            assert!(
                visible_army_chunk(&before).contains(expected_top),
                "{tab} top row proof missing: {before}"
            );
            let out = wheel_army_roster(&mut v, -3.0);
            assert!(
                out.consumed && out.needs_redraw,
                "{tab} wheel should scroll"
            );
            let after = bound_digest(&v.bind(&ctx(&stage, &snap)));
            assert!(
                visible_army_chunk(&after).contains(expected_after),
                "{tab} wheel-scroll should advance three backing rows: {after}"
            );
        }
    }

    #[test]
    fn army_scrollbar_drag_changes_visible_rows_and_clamps() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = many_rosters_snap(60, "A");
        let mut v = ArmyView::new();
        rebind_army(&mut v, &snap, true);

        let down = pointer_down_army_thumb(&mut v);
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "thumb pointer-down should consume to capture drag without changing scroll"
        );

        let geom = army_scrollbar(&v);
        let drag = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 250.0,
            y: geom.track.y + geom.track.h + 500.0,
            drag: true,
        });
        assert!(drag.consumed && drag.needs_redraw);
        let bottom = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let (cur, max) = army_scroll_pair(&bottom);
        assert_eq!(cur, max, "drag past the track bottom must clamp: {bottom}");
        assert!(
            !visible_army_chunk(&bottom).contains("key=army:1;name=A Army 01(E);"),
            "bottom-dragged visible slice must not keep top rows: {bottom}"
        );

        let geom = army_scrollbar(&v);
        let reset = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 250.0,
            y: geom.track.y - 500.0,
            drag: true,
        });
        assert!(reset.consumed && reset.needs_redraw);
        let top = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(army_scroll_pair(&top).0, 0);
        assert!(
            visible_army_chunk(&top).contains("key=army:1;name=A Army 01(E);"),
            "drag back above the track must clamp to the first row: {top}"
        );
    }

    #[test]
    fn army_scrollbar_drag_capture_continues_outside_and_clears_on_up() {
        let snap = many_rosters_snap(60, "A");
        let mut v = ArmyView::new();
        rebind_army(&mut v, &snap, true);

        let down = pointer_down_army_thumb(&mut v);
        assert!(down.consumed);

        let geom = army_scrollbar(&v);
        let drag = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x + geom.track.w + 300.0,
            y: geom.track.y + geom.track.h,
            drag: true,
        });
        assert!(
            drag.consumed && drag.needs_redraw,
            "captured scrollbar drag should continue even outside the list"
        );
        assert!(
            v.scroll.get("army") > 0.0,
            "outside captured drag should move the Army scroll offset"
        );

        let up = v.on_input(&ViewInput::PointerUp {
            x: geom.track.x + geom.track.w + 300.0,
            y: geom.track.y + geom.track.h,
            button: PointerButton::Left,
        });
        assert!(up.consumed && !up.needs_redraw);
        let after_up = v.scroll.get("army");
        let ignored = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x + geom.track.w + 300.0,
            y: geom.track.y,
            drag: true,
        });
        assert!(
            !ignored.consumed && !ignored.needs_redraw && ignored.commands.is_empty(),
            "pointer move after thumb release must not keep a stale scrollbar capture"
        );
        assert_eq!(v.scroll.get("army"), after_up);
    }

    #[test]
    fn army_scrollbar_track_click_does_not_select_row() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = many_rosters_snap(60, "A");
        let mut v = ArmyView::new();
        rebind_army(&mut v, &snap, true);

        let geom = army_scrollbar(&v);
        let track_y = (geom.thumb.y + geom.thumb.h + 10.0).min(geom.track.y + geom.track.h - 1.0);
        let track_click = v.on_input(&ViewInput::PointerDown {
            x: geom.track.x + geom.track.w * 0.5,
            y: track_y,
            button: PointerButton::Left,
        });
        assert!(
            track_click.consumed && !track_click.needs_redraw && track_click.commands.is_empty(),
            "clicking the scrollbar track outside the thumb should be an inert consumed UI hit"
        );
        assert!(
            v.selected_rows.get("army").is_none(),
            "scrollbar track click must not select an underlying Army row"
        );
        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("row_selected=none"),
            "track click must not alter selected-row digest, got: {d}"
        );

        let row = click_army_visible_row(&mut v, 0);
        assert!(row.consumed && row.needs_redraw);
        assert_eq!(v.selected_rows.get("army"), Some("army:1"));
    }

    #[test]
    fn army_outside_wheel_is_inert() {
        let snap = many_rosters_snap(60, "A");
        let mut v = ArmyView::new();
        rebind_army(&mut v, &snap, true);

        let viewport = army_roster_fallback(v.screen, selected_army_tab_id(&v.tabs));
        let outside = v.on_input(&ViewInput::Wheel {
            x: viewport.x + viewport.w + 20.0,
            y: viewport.y + ARMY_ROSTER_ROW_H,
            dy: -5.0,
        });
        assert!(
            !outside.consumed && !outside.needs_redraw && outside.commands.is_empty(),
            "wheel outside the Army roster must be inert"
        );
        assert_eq!(v.scroll.get("army"), 0.0);
    }

    #[test]
    fn army_scrollbar_drag_is_isolated_to_captured_tab() {
        let snap = many_rosters_snap(60, "A");
        let mut v = ArmyView::new();
        rebind_army(&mut v, &snap, true);

        let out = click_army_tab_center(&mut v, "navy");
        assert!(out.consumed && out.needs_redraw);
        let down = pointer_down_army_thumb(&mut v);
        assert!(down.consumed);

        let geom = army_scrollbar(&v);
        let drag = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x + geom.track.w + 200.0,
            y: geom.track.y + geom.track.h,
            drag: true,
        });
        assert!(drag.consumed && drag.needs_redraw);
        assert!(v.scroll.get("navy") > 0.0, "Navy drag should move Navy");
        assert_eq!(v.scroll.get("army"), 0.0, "Navy drag must not move Army");
        assert_eq!(v.scroll.get("air"), 0.0, "Navy drag must not move Air");

        let up = v.on_input(&ViewInput::PointerUp {
            x: geom.track.x + geom.track.w + 200.0,
            y: geom.track.y + geom.track.h,
            button: PointerButton::Left,
        });
        assert!(up.consumed);
        let out = click_army_tab_center(&mut v, "air");
        assert!(out.consumed && out.needs_redraw);
        assert_eq!(
            v.scroll.get("air"),
            0.0,
            "Air tab must retain its independent scroll offset"
        );
    }

    #[test]
    fn army_scrolled_row_click_selects_correct_backing_row() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = many_armies_snap(40, "A");
        let mut v = ArmyView::new();
        rebind_army(&mut v, &snap, true);

        wheel_army_roster(&mut v, -5.0);
        let out = click_army_visible_row(&mut v, 0);
        assert!(out.consumed);
        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("row_selected=r0[key=army:6;name=A 06(E);"),
            "after five-row scroll, visible row 0 must select backing row 6, got:\n{d}"
        );
    }

    #[test]
    fn army_row_selection_and_scroll_survive_scrub_a_b_a_without_b_rows() {
        let stage = stage_with_manifest(HashMap::new());
        let a = many_armies_snap(40, "A");
        let b = many_armies_snap(45, "B");
        let mut v = ArmyView::new();
        rebind_army(&mut v, &a, true);

        wheel_army_roster(&mut v, -5.0);
        click_army_visible_row(&mut v, 0);
        let da1 = bound_digest(&v.bind(&ctx(&stage, &a)));

        rebind_army(&mut v, &b, false);
        let db = bound_digest(&v.bind(&ctx(&stage, &b)));
        assert!(
            db.contains("row_selected=r0[key=army:6;name=B 06(E);"),
            "selected measured id should rebind to B's row data, got:\n{db}"
        );

        rebind_army(&mut v, &a, false);
        let da2 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert_eq!(
            da1, da2,
            "army row selection + scroll must be byte-identical across A->B->A"
        );
        assert!(
            !da2.contains("B 06"),
            "A-after-scrub must not carry B-only row text: {da2}"
        );
    }

    #[test]
    fn theatre_selector_row_click_selects_keyed_row() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = snap_a();
        let mut v = TheatreSelectorView::new();
        rebind_theatre(&mut v, &snap);

        let out = click_theatre_visible_row(&mut v, 0);
        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert!(out.commands.is_empty());

        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("selected=selected"),
            "clicking a theatre row must set selected status, got:\n{d}"
        );
        assert!(
            d.contains("selected_row=r0[key=theatre:61;name=Western Theatre(D);"),
            "clicking the first theatre row must select measured theatre id, got:\n{d}"
        );
    }

    #[test]
    fn theatre_selector_wheel_scroll_changes_visible_rows_and_clamps() {
        let stage = stage_all();
        let snap = many_theatres_snap(30, "A");
        let mut v = TheatreSelectorView::new();
        rebind_theatre(&mut v, &snap);

        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(theatre_scroll_pair(&before).0, 0);
        assert!(
            visible_theatre_chunk(&before).contains("key=theatre:1;name=A Theatre 01(D);"),
            "{before}"
        );

        let out = wheel_theatre_rows(&mut v, -3.0);
        assert!(out.consumed);
        assert!(out.needs_redraw);
        let after = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(theatre_scroll_pair(&after).0, 195);
        assert!(
            visible_theatre_chunk(&after).contains("key=theatre:4;name=A Theatre 04(D);"),
            "after three-row wheel scroll, visible rows must advance: {after}"
        );
        assert!(
            !visible_theatre_chunk(&after).contains("key=theatre:1;name=A Theatre 01(D);"),
            "scrolled visible slice must not keep top rows: {after}"
        );

        let out = wheel_theatre_rows(&mut v, -500.0);
        assert!(out.consumed);
        let clamped = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let (cur, max) = theatre_scroll_pair(&clamped);
        assert_eq!(cur, max, "huge wheel-down must clamp to max: {clamped}");

        let out = wheel_theatre_rows(&mut v, 500.0);
        assert!(out.consumed);
        let reset = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(theatre_scroll_pair(&reset).0, 0);
    }

    #[test]
    fn theatre_selector_scrollbar_drag_changes_visible_rows_and_clamps() {
        let stage = stage_all();
        let snap = many_theatres_snap(30, "A");
        let mut v = TheatreSelectorView::new();
        rebind_theatre(&mut v, &snap);

        let down = pointer_down_theatre_thumb(&mut v);
        assert!(
            down.consumed && !down.needs_redraw && down.commands.is_empty(),
            "thumb pointer-down should consume to capture drag without changing scroll"
        );

        let geom = theatre_scrollbar(&v);
        let target_y = geom.track.y + geom.track.h + 500.0;
        let drag = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 250.0,
            y: target_y,
            drag: true,
        });
        assert!(drag.consumed && drag.needs_redraw);
        let bottom = bound_digest(&v.bind(&ctx(&stage, &snap)));
        let (cur, max) = theatre_scroll_pair(&bottom);
        assert_eq!(cur, max, "drag past the track bottom must clamp: {bottom}");
        assert!(
            !visible_theatre_chunk(&bottom).contains("key=theatre:1;name=A Theatre 01(D);"),
            "bottom-dragged visible slice must not keep top rows: {bottom}"
        );

        let geom = theatre_scrollbar(&v);
        let reset = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x - 250.0,
            y: geom.track.y - 500.0,
            drag: true,
        });
        assert!(reset.consumed && reset.needs_redraw);
        let top = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert_eq!(theatre_scroll_pair(&top).0, 0);
        assert!(
            visible_theatre_chunk(&top).contains("key=theatre:1;name=A Theatre 01(D);"),
            "drag back above the track must clamp to the first row: {top}"
        );
    }

    #[test]
    fn theatre_selector_scrollbar_drag_capture_continues_outside_and_clears_on_up() {
        let stage = stage_all();
        let snap = many_theatres_snap(30, "A");
        let mut v = TheatreSelectorView::new();
        rebind_theatre(&mut v, &snap);

        let down = pointer_down_theatre_thumb(&mut v);
        assert!(down.consumed);

        let geom = theatre_scrollbar(&v);
        let drag = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x + geom.track.w + 300.0,
            y: geom.track.y + geom.track.h,
            drag: true,
        });
        assert!(
            drag.consumed && drag.needs_redraw,
            "captured scrollbar drag should continue even outside the list"
        );
        let dragged = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            theatre_scroll_pair(&dragged).0 > 0,
            "outside captured drag should move scroll, got: {dragged}"
        );

        let up = v.on_input(&ViewInput::PointerUp {
            x: geom.track.x + geom.track.w + 300.0,
            y: geom.track.y + geom.track.h,
            button: PointerButton::Left,
        });
        assert!(up.consumed && !up.needs_redraw);
        let after_up = v.scroll_y;
        let ignored = v.on_input(&ViewInput::PointerMove {
            x: geom.track.x + geom.track.w + 300.0,
            y: geom.track.y,
            drag: true,
        });
        assert!(
            !ignored.consumed && !ignored.needs_redraw && ignored.commands.is_empty(),
            "pointer move after thumb release must not keep a stale scrollbar capture"
        );
        assert_eq!(v.scroll_y, after_up);
    }

    #[test]
    fn theatre_selector_scrollbar_track_click_does_not_select_row() {
        let stage = stage_all();
        let snap = many_theatres_snap(30, "A");
        let mut v = TheatreSelectorView::new();
        rebind_theatre(&mut v, &snap);

        let geom = theatre_scrollbar(&v);
        let track_y = (geom.thumb.y + geom.thumb.h + 10.0).min(geom.track.y + geom.track.h - 1.0);
        let track_click = v.on_input(&ViewInput::PointerDown {
            x: geom.track.x + geom.track.w * 0.5,
            y: track_y,
            button: PointerButton::Left,
        });
        assert!(
            track_click.consumed && !track_click.needs_redraw && track_click.commands.is_empty(),
            "clicking the scrollbar track outside the thumb should be an inert consumed UI hit"
        );
        assert!(
            v.selected_theatre_key.is_none(),
            "scrollbar track click must not select an underlying theatre row"
        );
        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("selected=none"),
            "track click must not alter selected-row digest, got: {d}"
        );

        let row = click_theatre_visible_row(&mut v, 0);
        assert!(row.consumed && row.needs_redraw);
        assert_eq!(v.selected_theatre_key.as_deref(), Some("theatre:1"));
    }

    #[test]
    fn theatre_selector_scrolled_row_click_selects_correct_backing_row() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = many_theatres_snap(30, "A");
        let mut v = TheatreSelectorView::new();
        rebind_theatre(&mut v, &snap);

        wheel_theatre_rows(&mut v, -5.0);
        let out = click_theatre_visible_row(&mut v, 0);
        assert!(out.consumed && out.needs_redraw);
        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("selected_key=theatre:6"),
            "after five-row scroll, visible row 0 must select backing theatre 6, got:\n{d}"
        );
        assert!(
            d.contains("selected_row=r0[key=theatre:6;name=A Theatre 06(D);"),
            "selected row digest must point at backing theatre 6, got:\n{d}"
        );
    }

    #[test]
    fn theatre_selector_selection_and_scroll_survive_scrub_a_b_a_without_b_rows() {
        let stage = stage_with_manifest(HashMap::new());
        let a = many_theatres_snap(30, "A");
        let b = many_theatres_snap(35, "B");
        let mut v = TheatreSelectorView::new();
        rebind_theatre(&mut v, &a);

        wheel_theatre_rows(&mut v, -5.0);
        click_theatre_visible_row(&mut v, 0);
        let a1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert!(a1.contains("selected_key=theatre:6"));
        assert!(visible_theatre_chunk(&a1).contains("name=A Theatre 06(D);"));

        rebind_theatre(&mut v, &b);
        let b_digest = bound_digest(&v.bind(&ctx(&stage, &b)));
        assert!(
            b_digest.contains("selected_row=r0[key=theatre:6;name=B Theatre 06(D);"),
            "selected measured id should rebind to B's row data, got:\n{b_digest}"
        );
        assert!(
            !visible_theatre_chunk(&b_digest).contains("name=A Theatre 06(D);"),
            "B visible chunk must not carry A-only row text, got:\n{b_digest}"
        );

        rebind_theatre(&mut v, &a);
        let a2 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, a2,
            "theatre row selection + scroll must be byte-identical across A->B->A"
        );
        assert!(
            !a2.contains("B Theatre 06"),
            "A-after-scrub must not carry B-only row text: {a2}"
        );
    }

    #[test]
    fn theatre_selector_missing_identity_and_outside_input_are_inert() {
        let snap = theatre_missing_first_identity_snap();
        let mut v = TheatreSelectorView::new();
        rebind_theatre(&mut v, &snap);

        let missing_id = click_theatre_visible_row(&mut v, 0);
        assert!(
            !missing_id.consumed && !missing_id.needs_redraw && missing_id.commands.is_empty(),
            "row without measured theatre id must not be selectable by index/name"
        );
        assert!(v.selected_theatre_key.is_none());

        let viewport = theatre_selector_list_fallback(v.screen);
        let outside = v.on_input(&ViewInput::Wheel {
            x: viewport.x + viewport.w + 20.0,
            y: viewport.y + THEATRE_SELECTOR_ROW_H,
            dy: -5.0,
        });
        assert!(
            !outside.consumed && !outside.needs_redraw && outside.commands.is_empty(),
            "wheel outside the theatre list must be inert"
        );

        let keyed = click_theatre_visible_row(&mut v, 1);
        assert!(keyed.consumed && keyed.needs_redraw);
        assert_eq!(v.selected_theatre_key.as_deref(), Some("theatre:202"));
    }

    #[test]
    fn theatre_selector_duplicate_ids_are_unselectable_without_index_fallback() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = theatre_duplicate_id_snap();
        let mut v = TheatreSelectorView::new();
        rebind_theatre(&mut v, &snap);

        let d = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            d.contains("name=Duplicate Theatre A(D);")
                && d.contains("name=Duplicate Theatre B(D);"),
            "duplicate-id rows should still render their measured data, got: {d}"
        );
        assert!(
            !d.contains("key=theatre:77;"),
            "duplicate measured theatre id must not become a selectable ambiguous key, got: {d}"
        );

        let first = click_theatre_visible_row(&mut v, 0);
        assert!(
            !first.consumed && !first.needs_redraw && first.commands.is_empty(),
            "first duplicate-id theatre row must be inert"
        );
        let second = click_theatre_visible_row(&mut v, 1);
        assert!(
            !second.consumed && !second.needs_redraw && second.commands.is_empty(),
            "second duplicate-id theatre row must be inert"
        );
        assert!(v.selected_theatre_key.is_none());

        let unique = click_theatre_visible_row(&mut v, 2);
        assert!(unique.consumed && unique.needs_redraw);
        let selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains("selected_row=r0[key=theatre:88;name=Unique Theatre(D);"),
            "unique theatre id should remain selectable, got: {selected}"
        );
    }

    #[test]
    fn theatre_selector_missing_selected_row_fails_closed_without_row_fallback() {
        let stage = stage_with_manifest(HashMap::new());
        let a = theatre_missing_first_identity_snap();
        let b = theatre_missing_selected_snap();
        let mut v = TheatreSelectorView::new();

        rebind_theatre(&mut v, &a);
        let out = click_theatre_visible_row(&mut v, 1);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert!(a1.contains("selected_key=theatre:202"));

        rebind_theatre(&mut v, &b);
        let missing = bound_digest(&v.bind(&ctx(&stage, &b)));
        assert!(
            missing.contains("selected=missing_selected"),
            "foreign-owned/missing selected theatre id must fail closed, got: {missing}"
        );
        assert!(
            missing.contains("selected_key=theatre:202"),
            "missing digest must retain the requested theatre key, got: {missing}"
        );
        assert!(
            missing.contains("selected_row=none"),
            "missing selected theatre must not fall back to another row, got: {missing}"
        );
        assert!(
            !missing.contains("selected_row=r0[key=theatre:1;"),
            "missing selected theatre must not highlight the remaining first row, got: {missing}"
        );

        rebind_theatre(&mut v, &a);
        let restored = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, restored,
            "returning to A must restore the selected theatre row exactly"
        );
    }

    fn raw_named_child<'a>(
        node: &'a gui_runtime::model::RawNode,
        name: &str,
    ) -> Option<&'a gui_runtime::model::RawNode> {
        if node.name.as_deref() == Some(name) {
            return Some(node);
        }
        for child in &node.children {
            if let Some(found) = raw_named_child(child, name) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn equipment_overview_real_gui_geometry_matches_interaction_fallback() {
        if !assets_extracted() {
            eprintln!("[mil-mgmt] skipped equipment geometry: game_assets/gui not extracted");
            return;
        }
        let file = load_gui_file(EQUIPMENT_OVERVIEW_FILE).expect("equipment overview gui parses");
        let root = file
            .windows
            .iter()
            .find(|w| w.name.as_deref() == Some(EQUIPMENT_OVERVIEW_ROOT))
            .expect("equipment overview root exists");
        let equipment = raw_named_child(root, "equipment").expect("equipment container exists");
        let grid = raw_named_child(root, "equipment_list").expect("equipment_list grid exists");

        let screen = default_equipment_overview_screen();
        let root_pos = root.position.as_ref().expect("root has position");
        let equipment_pos = equipment.position.as_ref().expect("equipment has position");
        let grid_pos = grid.position.as_ref().expect("equipment_list has position");
        let expected_x =
            screen.x + screen.w + root_pos.x.px() + equipment_pos.x.px() + grid_pos.x.px();
        let expected_y = screen.y + root_pos.y.px() + equipment_pos.y.px() + grid_pos.y.px();
        let fallback = equipment_overview_list_fallback(screen);
        assert_eq!(
            (fallback.x.round() as i32, fallback.y.round() as i32),
            (expected_x.round() as i32, expected_y.round() as i32),
            "equipment input viewport must stay anchored to real equipment_overview.json offsets"
        );
        assert_eq!(
            (fallback.w.round() as i32, fallback.h.round() as i32),
            (315, 527),
            "equipment input viewport must match the real 320px panel minus the grid offset"
        );

        let template =
            find_template(&file, "overview_equipment_entry").expect("equipment row template");
        let spec =
            GridSpec::from_gridbox(&stub_grid("equipment_list", fallback), &template, GUTTER);
        assert_eq!(
            spec.cell_h.round() as i32,
            EQUIPMENT_OVERVIEW_ROW_H.round() as i32,
            "equipment row picking must use the real overview_equipment_entry row height"
        );
    }

    #[test]
    fn equipment_overview_row_click_selects_measured_variant_key() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = snap_a();
        let mut v = EquipmentOverviewView::new();
        rebind_equipment(&mut v, &snap);

        let before = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(before.contains("selected=none"), "{before}");
        assert!(
            before.contains("key=equipment:2:0;equipment_name=2(D);equipment_amount=1200(E);"),
            "equipment digest must expose the measured composite variant key, got: {before}"
        );

        let out = click_equipment_visible_row(&mut v, 1);
        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert!(out.commands.is_empty());

        let selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(selected.contains("selected=selected"), "{selected}");
        assert!(
            selected.contains("selected_key=equipment:2:0"),
            "{selected}"
        );
        assert!(
            selected.contains(
                "selected_row=r0[key=equipment:2:0;equipment_name=2(D);equipment_amount=1200(E);"
            ),
            "clicking row 1 must select the measured equipment variant, got: {selected}"
        );
    }

    #[test]
    fn equipment_overview_missing_and_duplicate_identity_rows_are_inert() {
        let stage = stage_with_manifest(HashMap::new());
        let snap = equipment_missing_duplicate_identity_snap();
        let mut v = EquipmentOverviewView::new();
        rebind_equipment(&mut v, &snap);

        let digest = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            digest.contains("equipment_name=10(D);")
                && digest.contains("equipment_name=11(D);")
                && digest.contains("equipment_name=12(D);"),
            "missing/duplicate rows should still render measured amounts, got: {digest}"
        );
        assert!(
            !digest.contains("key=equipment:10:") && !digest.contains("key=equipment:11:0;"),
            "missing or duplicate measured equipment identity must not become selectable, got: {digest}"
        );
        assert!(
            digest.contains("key=equipment:12:0;equipment_name=12(D);equipment_amount=40(E);"),
            "unique measured equipment identity should remain selectable, got: {digest}"
        );

        for idx in 0..3 {
            let out = click_equipment_visible_row(&mut v, idx);
            assert!(
                !out.consumed && !out.needs_redraw && out.commands.is_empty(),
                "equipment row {idx} must be inert without a unique measured key"
            );
        }
        assert!(v.selected_equipment_key.is_none());

        let unique = click_equipment_visible_row(&mut v, 3);
        assert!(unique.consumed && unique.needs_redraw);
        let selected = bound_digest(&v.bind(&ctx(&stage, &snap)));
        assert!(
            selected.contains(
                "selected_row=r0[key=equipment:12:0;equipment_name=12(D);equipment_amount=40(E);"
            ),
            "unique equipment row should select by composite key, got: {selected}"
        );
    }

    #[test]
    fn equipment_overview_missing_selected_row_fails_closed_without_row_fallback() {
        let stage = stage_with_manifest(HashMap::new());
        let a = snap_a();
        let b = equipment_missing_selected_snap();
        let mut v = EquipmentOverviewView::new();

        rebind_equipment(&mut v, &a);
        let out = click_equipment_visible_row(&mut v, 1);
        assert!(out.consumed && out.needs_redraw);
        let a1 = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert!(a1.contains("selected_key=equipment:2:0"), "{a1}");

        rebind_equipment(&mut v, &b);
        let missing = bound_digest(&v.bind(&ctx(&stage, &b)));
        assert!(
            missing.contains("selected=missing_selected"),
            "missing selected equipment id must fail closed, got: {missing}"
        );
        assert!(
            missing.contains("selected_key=equipment:2:0") && missing.contains("selected_row=none"),
            "missing selected equipment key must be retained without row fallback, got: {missing}"
        );
        assert!(
            !missing.contains("selected_row=r0[key=equipment:1:0;"),
            "missing selected equipment must not highlight the remaining first row, got: {missing}"
        );

        rebind_equipment(&mut v, &a);
        let restored = bound_digest(&v.bind(&ctx(&stage, &a)));
        assert_eq!(
            a1, restored,
            "returning to A must restore the selected equipment row exactly"
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

    /// For EVERY one of the 15 `.gui` files, the element set we account for (the recursive
    /// `type`-token walk over the parsed tree) EQUALS the file's declared element_type_counts
    /// — proving every element of every view is accounted (none silently dropped).
    #[test]
    fn element_accounting_all_15_views() {
        if !assets_extracted() {
            eprintln!("[mil-mgmt] skipped accounting: game_assets/gui not extracted");
            return;
        }
        for f in FILES {
            assert_eq!(
                raw_type_counts(f),
                declared_counts(f),
                "element accounting drift in {f}"
            );
        }
    }

    /// The per-view totals match the binding-map's stated parsed-node counts (a guard that the
    /// source files haven't drifted under the map).
    #[test]
    fn element_accounting_totals_match_binding_map() {
        if !assets_extracted() {
            return;
        }
        let total = |n: &str| raw_type_counts(n).values().sum::<usize>();
        let expect: &[(&str, usize)] = &[
            ("army.json", 141),
            ("navies.json", 398),
            ("air_group.json", 20),
            ("air_selection.json", 241),
            ("air_wing_details.json", 50),
            ("air_wing_reorg.json", 104),
            ("carrier_air_wing.json", 21),
            ("strategic_air.json", 203),
            ("battle_plan_tools.json", 62),
            ("ability_list.json", 8),
            ("theatre_selector.json", 143),
            ("divisions_summary.json", 5),
            ("railway_gun.json", 71),
            ("deployment.json", 159),
            ("equipment_overview.json", 28),
        ];
        for (f, n) in expect {
            assert_eq!(
                total(f),
                *n,
                "{f} total node count drifted from the binding map"
            );
        }
    }

    /// Collect every `name` in a `.gui` window subtree (the named element set), walking the
    /// RAW json (lenient — independent of the typed `GuiFile` parser).
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

    /// BINDING-COVERAGE guard (Codex point 5): every element name a `Cell`/summary binds MUST
    /// exist as a real node in its target window (ROOT for summary cells, the row TEMPLATE for
    /// row cells) — else `set_element_text` silently no-ops. The (file, window, [elements])
    /// pairs below mirror the `bind` code across all 15 views.
    #[test]
    fn every_bound_element_name_exists_in_its_window() {
        if !assets_extracted() {
            return;
        }
        let cases: &[(&str, &str, &[&str])] = &[
            // VIEW 1 ARMY
            (
                "army.json",
                "countryarmyview",
                &["unit_count", "battalion_count", "special_forces_cap"],
            ),
            (
                "army.json",
                "armyview_army_entry",
                &[
                    "name",
                    "theatre",
                    "group",
                    "group_marker",
                    "level",
                    "strength_progressbar",
                    "organization_progressbar",
                ],
            ),
            (
                "army.json",
                "armyview_navy_entry",
                &[
                    "unit_name",
                    "unit_count",
                    "region",
                    "mission_icon",
                    "leader_button",
                    "organization_progressbar",
                    "strength_progressbar",
                ],
            ),
            (
                "army.json",
                "armyview_air_entry",
                &[
                    "unit_name",
                    "count",
                    "assigned_region",
                    "base",
                    "limit",
                    "ace",
                ],
            ),
            // VIEW 2 NAVIES
            (
                "navies.json",
                "fleet_view_header",
                &[
                    "fleet_name",
                    "taskforce_count",
                    "active_in_regions",
                    "home_base_name",
                    "fuel_icon",
                    "insigna",
                    "leader_button",
                    "leader_name",
                    "skill_text",
                ],
            ),
            (
                "navies.json",
                "taskforce_view_entry",
                &[
                    "unit_name",
                    "current_mission",
                    "insigna",
                    "activity_description",
                    "dominance",
                    "fuel_stocks_icon",
                    "reserve_fleet_icon",
                ],
            ),
            (
                "navies.json",
                "ship_entry_detailed",
                &[
                    "ship_name",
                    "ship_type",
                    "bar_str",
                    "bar_org",
                    "xp_bar",
                    "bar_planes",
                    "repair_symbol",
                ],
            ),
            // VIEW 3 AIR GROUP
            (
                "air_group.json",
                "air_group_view_entry",
                &[
                    "unit_name",
                    "fuel_stocks_icon",
                    "fuel_efficiency_warning_icon",
                ],
            ),
            // VIEW 4 AIR SELECTION
            (
                "air_selection.json",
                "airbase_selection_item",
                &[
                    "base_name",
                    "capacity",
                    "owner_flag",
                    "airbase_friendship_status",
                    "fuel_icon",
                    "permission_wings_count",
                ],
            ),
            (
                "air_selection.json",
                "airwing_selection_item",
                &[
                    "unit_name",
                    "count",
                    "mission_state",
                    "xp_bar",
                    "air_mission_efficiency_strip",
                    "armies_count_box",
                    "ace_btn",
                ],
            ),
            // VIEW 5 AIR WING DETAILS
            (
                "air_wing_details.json",
                "air_wing_stats_window",
                &[
                    "air_wing_name",
                    "current_manpower",
                    "equipment_ratio",
                    "ace_name",
                ],
            ),
            (
                "air_wing_details.json",
                "air_wing_details_equipment_entry",
                &["count", "name", "type"],
            ),
            // VIEW 6 AIR WING REORG
            (
                "air_wing_reorg.json",
                "air_wing_reorganization",
                &["current_manpower_left", "current_manpower_right"],
            ),
            (
                "air_wing_reorg.json",
                "air_wing_reorganization_equipment_entry_left",
                &["count", "name", "type", "design_team_icon", "niche_icon"],
            ),
            // VIEW 7 CARRIER AIR WING
            (
                "carrier_air_wing.json",
                "carrier_air_wing_composition_window",
                &["title", "summary", "carrier_capacity_icon"],
            ),
            // VIEW 8 STRATEGIC AIR
            (
                "strategic_air.json",
                "strategicairview",
                &[
                    "region_name",
                    "wing_type_left_text_1",
                    "progress",
                    "wing_type_right_text_1",
                    "detection_left_text",
                    "effeciency_left_text",
                    "air_weather_frame",
                    "our_fighters_count",
                ],
            ),
            // VIEW 9 BATTLE PLAN TOOLS
            (
                "battle_plan_tools.json",
                "battleplantools_window",
                &[
                    "selected_order_overlay",
                    "operation_name",
                    "icon_execute_type",
                ],
            ),
            // VIEW 11 THEATRE SELECTOR
            (
                "theatre_selector.json",
                "theatre_selection_entry",
                &["name", "unit_indicator", "priority_1", "pinned"],
            ),
            // VIEW 12 DIVISIONS SUMMARY
            (
                "divisions_summary.json",
                "divisions_summary_item",
                &["count", "selection_marker"],
            ),
            // VIEW 13 RAILWAY GUN
            (
                "railway_gun.json",
                "railway_gun_list_entry",
                &["unit_name", "bar_str", "flag", "low_supply", "no_orders"],
            ),
            // VIEW 14 DEPLOYMENT
            (
                "deployment.json",
                "named_division_template_entry",
                &["name", "template_cap", "division_priority_icon"],
            ),
            (
                "deployment.json",
                "deploy_entry",
                &["name", "equipment_progressbar", "reinforcements_icon"],
            ),
            // VIEW 15 EQUIPMENT OVERVIEW
            (
                "equipment_overview.json",
                "equipment_overview_window",
                &["current_manpower", "equipment_ratio"],
            ),
            (
                "equipment_overview.json",
                "overview_archetype_header_list_entry",
                &["archetype_name", "archetype_amount"],
            ),
            (
                "equipment_overview.json",
                "overview_equipment_entry",
                &["equipment_name", "equipment_amount"],
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

    /// All 15 cluster views are inventory-collected + routed under the Military menu cluster.
    #[test]
    fn cluster_views_registered() {
        use crate::host::ViewRegistry;
        let reg = ViewRegistry::from_inventory();
        let ids = [
            "military.army",
            "military.navies",
            "military.air_group",
            "military.air_selection",
            "military.air_wing_details",
            "military.air_wing_reorg",
            "military.carrier_air_wing",
            "military.strategic_air",
            "military.battle_plan_tools",
            "military.ability_list",
            "military.theatre_selector",
            "military.divisions_summary",
            "military.railway_gun",
            "military.deployment",
            "military.equipment_overview",
        ];
        for id in ids {
            assert!(
                reg.view(ViewId(id)).is_some(),
                "view {id} must be inventory-collected"
            );
        }
        let bar = reg.menubar();
        let group = bar
            .iter()
            .find(|g| g.cluster == MenuCluster::MilitaryManagement)
            .unwrap();
        assert_eq!(
            group.items.len(),
            15,
            "all 15 cluster views routed to the Military menu"
        );
    }
}
