//! CLUSTER 2e — designers (5 views) [N4 module-slot layout + N2 tabs].
//!
//! The five in-game the game DESIGNER views, bound per `replay_ui/BINDING_MAP_designers.md`:
//!   * **tank**  — `tank_designer.json` ROOT `tank_designer_view` — a `land_vehicle`
//!     equipment variant: module slots + inline upgrade sliders + identity.
//!   * **plane** — `plane_designer.json` ROOT `plane_designer_view` — a `plane` variant
//!     (fixed + custom slot rows).
//!   * **division** — `division_designer.json` ROOT `countrydivisiondesignerview` — a
//!     division TEMPLATE: the regiment GRID + support column (N2 tabs: normal vs HQ).
//!   * **equipment (naval)** — `equipment_designer.json` ROOT `countryequipmentdesignerview`
//!     — a `ship` variant (custom + fixed slot rows + naval-dominance).
//!   * **equipment-upgrade** — `equipment_upgrade_designer.json` ROOT
//!     `equipment_upgrade_designer_window` — the named upgrade-track slider window.
//!
//! ## What is BOUND vs BADGED (the §G hard limit)
//! EXACT/derived (bound): the module SLOT layout (`designs.equipment_variants[V].modules`
//! / live `equipment_variants_live[V].modules`), the upgrade-slider LEVELS
//! (`upgrades{name:lvl}`), the division regiment GRID
//! (`division_templates[T].regiments[{type,x,y}]` + `support[]`), the design IDENTITY
//! (name / archetype / family / version / creator / obsolete / MIO design-team), and the
//! per-country experience (`military.xp.{army,navy,air}`). Module/upgrade NAMES + ICONS +
//! per-module STAT CONTRIBUTIONS resolve from the static DEF catalog
//! (`game_defs/out/catalogs.json` → `equipment_modules`/`equipment_archetypes`/
//! `equipment_upgrades`) → **derived (def)**.
//!
//! UNAVAILABLE (NEVER computed/fabricated): every derived COMBAT stat panel
//! (`base_stats`/`combat_stats`/`misc_stats`/`equipment_stats`/`adjuster_stats`), the
//! division stat chips (hardness/org/strength/speed/combat-width), every
//! `production_value`/`production_resources`/`refit_cost`, the naval-dominance value, the
//! HQ deploy cost/time + HQ modifiers. These are runtime serialize-computed with no
//! persisted backing field (BINDING_MAP §G), so they badge "—".
//!
//! ## RENDER: the REAL `.gui` + N4 + N2
//! `bind` resolves the REAL parsed game layout via [`resolve_gui`] (Stage's pre-parsed
//! `.gui` first; a cached cluster-local sanitized parse for the 3 designer files the shared
//! parser rejects; an empty root only if both fail) and lays it out — that is the faithful
//! chrome (backgrounds, labels, blueprint board, the static element set the accounting test
//! walks). The BOUND DATA is drawn ON TOP as an N4 [`widgets::designer::ModuleLayout`] (the
//! slot grid + module icons + per-slot stat contributions) plus the header atoms + the
//! stat-panel unavailable badges; the division designer adds an N2 [`widgets::tabs::TabBar`]
//! (normal vs HQ), and the equipment-upgrade window hit-tests its real `.gui` upgrade-row
//! controls as replay-disabled mutation no-ops. Every visible value is a [`widgets::Atom`] /
//! [`DisplayAtom`] — a blocked field draws the not-captured glyph, never a fabricated 0.
//!
//! ## NO-FABRICATION + the top-level `designs` domain
//! Designer data lives in the TOP-LEVEL `designs` snapshot domain (each
//! `division_templates[]` element carries its owning `country` TAG; `equipment_variants`
//! is keyed `"id:type"` and carries `creator`), NOT under `countries.<TAG>`/`states.<id>`.
//! `BindContext::country_field`/`state_field` (the cluster's audited Captured accessors)
//! structurally cannot address it, and `BindContext` has no top-level-domain Captured
//! accessor yet (the same wall `scripted_gui_overlays` documented for the `world` domain).
//! [`designs_domain`] is a NARROW local shim converting `Snapshot::domain("designs")` into
//! the SAME [`Captured`] Absent/Null/Measured trichotomy — so blocked/absent designs data
//! renders `unavailable`, never a fabricated default. It is intentionally tiny + domain-
//! specific and should be removed once a shared `BindContext::domain_field` lands. The
//! per-country experience reads through the normal `ctx.country_field` accessor.
//!
//! THIS FILE IS YOURS, cluster agent — edit ONLY here. Registration is distributed
//! through `inventory`, so adding/binding a view edits NO shared file.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;
use snapshot::Snapshot;

use gui_runtime::scene::Node;
use gui_runtime::{build_scene, layout, GuiFile, ManifestContent, Rect};

use bind::Fidelity;

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

use widgets::common::{AssetRef, Atom, DrawAssets, Fidelity as WFidelity, Reason};
use widgets::designer::{Module, ModuleLayout, Slot};
use widgets::tabs::{Tab, TabBar};

mod defs;
mod model;

use defs::DefCatalog;
use model::DesignerModel;

// =====================================================================================
// The NO-FABRICATION top-level `designs` domain shim (Codex-reviewed; see module docs).
// =====================================================================================

/// Local Captured accessor for the TOP-LEVEL `designs` domain.
///
/// Designer data is stored under `Snapshot::domain("designs")`, not under
/// `countries.<TAG>` or `states.<id>`, so `BindContext::country_field`/`state_field`
/// cannot address it and there is no shared top-level-domain Captured accessor yet.
/// This shim preserves the no-fabrication contract by converting the domain lookup into
/// the SAME [`Captured`] trichotomy (Absent / Null / Measured), so blocked or absent
/// designs data renders `unavailable` rather than a fabricated default. It is
/// intentionally narrow + domain-specific.
///
/// TODO: replace with a shared `BindContext::domain_field("designs", ..)` once one lands.
fn designs_domain(snapshot: &Snapshot) -> Captured<&Value> {
    match snapshot.domain("designs") {
        None => Captured::Absent,
        Some(Value::Null) => Captured::Null,
        Some(v) => Captured::Measured(v),
    }
}

/// The `equipment_variants` object out of the `designs` domain, if present+object.
fn equipment_variants(snapshot: &Snapshot) -> Option<&serde_json::Map<String, Value>> {
    match designs_domain(snapshot) {
        Captured::Measured(v) => v.get("equipment_variants").and_then(Value::as_object),
        _ => None,
    }
}

/// The `equipment_variants_live` object (the live module/upgrade leaf), if present+object.
fn equipment_variants_live(snapshot: &Snapshot) -> Option<&serde_json::Map<String, Value>> {
    match designs_domain(snapshot) {
        Captured::Measured(v) => v.get("equipment_variants_live").and_then(Value::as_object),
        _ => None,
    }
}

/// The `division_templates` array out of the `designs` domain, if present+array.
fn division_templates(snapshot: &Snapshot) -> Option<&[Value]> {
    match designs_domain(snapshot) {
        Captured::Measured(v) => v
            .get("division_templates")
            .and_then(Value::as_array)
            .map(|a| a.as_slice()),
        _ => None,
    }
}

/// The `family` filter for a component designer view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    LandVehicle,
    Plane,
    Ship,
}

impl Family {
    fn token(self) -> &'static str {
        match self {
            Family::LandVehicle => "land_vehicle",
            Family::Plane => "plane",
            Family::Ship => "ship",
        }
    }
    /// The per-country experience channel this designer spends (army/navy/air).
    fn xp_field(self) -> &'static str {
        match self {
            Family::LandVehicle => "military.xp.army",
            Family::Plane => "military.xp.air",
            Family::Ship => "military.xp.navy",
        }
    }
    fn component_root_size(self) -> (f32, f32) {
        match self {
            Family::LandVehicle | Family::Plane => (1091.0, 542.0),
            Family::Ship => (1097.0, 542.0),
        }
    }
}

/// Pick the FOCUSED variant `V` for the observed country: the first `equipment_variants`
/// entry whose `creator == tag` and `family == fam`. Returns `(key, &variant_obj)` or
/// `None` when none exist for this country/family (→ the view badges all variant binds
/// `unavailable`, never fabricated). Deterministic: keys are iterated sorted so scrub is
/// byte-identical.
fn focused_variant<'a>(
    snapshot: &'a Snapshot,
    tag: &str,
    fam: Family,
) -> Option<(String, &'a Value)> {
    let obj = equipment_variants(snapshot)?;
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();
    for k in keys {
        let v = &obj[k];
        let creator = v.get("creator").and_then(Value::as_str);
        let family = v.get("family").and_then(Value::as_str);
        if creator == Some(tag) && family == Some(fam.token()) {
            return Some((k.clone(), v));
        }
    }
    None
}

/// Pick the FOCUSED division template `T` for the observed country: the first
/// `division_templates[]` element whose `country == tag`. Deterministic (input order).
fn focused_template<'a>(snapshot: &'a Snapshot, tag: &str) -> Option<&'a Value> {
    division_templates(snapshot)?
        .iter()
        .find(|t| t.get("country").and_then(Value::as_str) == Some(tag))
}

// =====================================================================================
// Shared format helpers (only ever applied to a Measured value via Captured).
// =====================================================================================

const DASH: &str = "—";

fn fmt_int(v: &Value) -> String {
    match v.as_f64() {
        Some(f) => format!("{}", f.trunc() as i64),
        None => match v.as_str() {
            Some(s) => s.to_string(),
            None => DASH.to_string(),
        },
    }
}

fn fmt_token(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => DASH.to_string(),
    }
}

/// Convert a host [`DisplayAtom`] into a widgets [`Atom`] WITHOUT a host→widgets dep
/// cycle, preserving the trichotomy (an Unavailable atom can never smuggle a number).
fn to_watom(a: &DisplayAtom) -> Atom {
    match a {
        DisplayAtom::Text { text, fidelity } => {
            let wf = match fidelity {
                Fidelity::Exact => WFidelity::Exact,
                Fidelity::Derived => WFidelity::Derived,
                Fidelity::Unavailable => WFidelity::Unavailable,
            };
            Atom::from_parts(text.clone(), wf, Reason::Blocked)
        }
        DisplayAtom::Unavailable { reason, .. } => {
            let r = match reason {
                UnavailableReason::Absent => Reason::Absent,
                UnavailableReason::Null => Reason::Null,
                UnavailableReason::Blocked => Reason::Blocked,
            };
            Atom::unavailable_because(r)
        }
    }
}

/// Read a value field of an object as a [`Captured`] (preserving the trichotomy at leaf).
fn obj_field<'a>(obj: &'a Value, key: &str) -> Captured<&'a Value> {
    crate::host::context::classify_value(obj.get(key))
}

fn empty_gui() -> GuiFile {
    GuiFile::parse(r#"{"view":"empty","windows":[]}"#).expect("empty gui parses")
}

// =====================================================================================
// Resolve the REAL `.gui` (Codex A-prime): Stage → cached sanitized fallback → empty.
// =====================================================================================

/// Resolve the view's REAL `.gui` into a parsed [`GuiFile`], rendering the authentic game
/// layout. Order (Codex A-prime): (1) the binary's pre-parsed `Stage.gui_files`; (2) a
/// CLUSTER-LOCAL, CACHED sanitized parse of the raw `.gui` for the 3 designer files the
/// SHARED `gui_runtime` parser rejects (they carry Clausewitz `@named` POSITION anchor
/// tokens — e.g. `"@fixed_btn_mod_row_0"` — where the typed parser wants an `f32`); (3)
/// an empty root only if BOTH fail. The fallback keeps the REAL `.gui` authoritative
/// (real file, root, windows, elements, names, backgrounds) — only unsupported LAYOUT
/// anchor offsets degrade (to 0.0), never a bound DATUM (no-fabrication is about data
/// values, not layout coords). The parse is memoised (the `.gui` is static), so `bind`
/// stays cheap + effectively pure after first load.
fn resolve_gui(ctx: &BindContext, file: &'static str) -> GuiFile {
    if let Some(g) = ctx.stage.gui_file(file) {
        return g.clone();
    }
    match sanitized_gui(ctx, file) {
        Some(g) => g,
        None => empty_gui(),
    }
}

/// The per-file cache of the sanitized fallback parse (success AND failure cached, so we
/// never re-IO/re-warn each tick). Keyed by file name.
fn sanitized_cache() -> &'static Mutex<HashMap<&'static str, Option<GuiFile>>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Option<GuiFile>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Read the raw `.gui` JSON for `file`, surgically rewrite any `@named` POSITION-anchor
/// string into a numeric fallback the shared parser accepts, then `GuiFile::parse`. Cached
/// (success+failure). READ-ONLY; a miss/parse-fail caches `None` (→ empty root).
fn sanitized_gui(ctx: &BindContext, file: &'static str) -> Option<GuiFile> {
    {
        let cache = sanitized_cache().lock().ok()?;
        if let Some(entry) = cache.get(file) {
            return entry.clone();
        }
    }
    let parsed = load_and_sanitize(&ctx.stage.assets_dir, file);
    if let Ok(mut cache) = sanitized_cache().lock() {
        cache.insert(file, parsed.clone());
    }
    parsed
}

/// Locate + read + sanitize + parse the raw `.gui`. Tries the stage's `assets_dir/gui/`
/// first, then the repo `game_assets/gui/` (derived from the crate dir) so a test/headless
/// run resolves it too.
fn load_and_sanitize(assets_dir: &std::path::Path, file: &str) -> Option<GuiFile> {
    let mut candidates: Vec<std::path::PathBuf> = vec![assets_dir.join("gui").join(file)];
    if let Some(repo) = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
    {
        candidates.push(repo.join("game_assets").join("gui").join(file));
    }
    let txt = candidates
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())?;
    let mut v: Value = serde_json::from_str(&txt).ok()?;
    sanitize_named_positions(&mut v);
    GuiFile::parse(&v.to_string()).ok()
}

fn repo_game_assets_dir() -> Option<std::path::PathBuf> {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|repo| repo.join("game_assets"))
}

/// Recursively rewrite a `position.{x,y}` value that is a STRING starting with `@` into
/// `0.0`. Surgical: ONLY the `position` object's `x`/`y` string-`@` values are touched (a
/// layout anchor the shared parser cannot resolve), never any data/text field. This adapts
/// unsupported Clausewitz layout syntax into the numeric shape the parser requires — it is
/// a layout fallback, not a fabricated value.
fn sanitize_named_positions(v: &mut Value) {
    match v {
        Value::Object(map) => {
            if let Some(Value::Object(pos)) = map.get_mut("position") {
                for axis in ["x", "y"] {
                    if let Some(val) = pos.get_mut(axis) {
                        if matches!(val, Value::String(s) if s.starts_with('@')) {
                            *val = Value::from(0.0);
                        }
                    }
                }
            }
            for (_k, child) in map.iter_mut() {
                sanitize_named_positions(child);
            }
        }
        Value::Array(arr) => {
            for child in arr.iter_mut() {
                sanitize_named_positions(child);
            }
        }
        _ => {}
    }
}

/// Build + lay out the REAL `.gui` ROOT for the view (the faithful chrome). Mirrors the
/// production cluster's `build_root`: a fresh scene per tick, laid out at the screen rect.
fn build_root(ctx: &BindContext, file: &GuiFile, root_name: &str) -> Vec<Node> {
    let stage = ctx.stage;
    let cm = ManifestContent {
        manifest: &stage.manifest,
        measure: |_t, _f| None,
    };
    let mut roots: Vec<Node> = build_scene(file, &stage.manifest, &stage.loc)
        .into_iter()
        .filter(|w| w.name == root_name)
        .collect();
    for root in roots.iter_mut() {
        layout(root, ctx.screen, None, &cm);
    }
    roots
}

/// The on-screen area the N4 module/regiment overlay + header + stat badges draw into,
/// derived from the laid-out ROOT (or a default if the root is empty/headless).
fn overlay_area(roots: &[Node], screen: Rect) -> Rect {
    let base = roots
        .first()
        .map(|r| r.rect)
        .filter(|r| r.w > 0.0 && r.h > 0.0)
        .unwrap_or(screen);
    Rect {
        x: base.x + 12.0,
        y: base.y + 64.0,
        w: (base.w - 24.0).max(0.0),
        h: (base.h - 76.0).max(0.0),
    }
}

const COMPONENT_HEADER_ROWS: usize = 7;

#[derive(Debug, Clone)]
struct ComponentPickModel {
    modules: ModuleLayout,
    grid_area: Rect,
    disabled_area: Rect,
    controls: Vec<ComponentControlHit>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ComponentControlHit {
    id: String,
    rect: Rect,
}

#[derive(Debug, Clone, Default)]
struct ComponentInteraction {
    selected_slot: Option<String>,
    hovered_slot: Option<String>,
    pick_model: Option<ComponentPickModel>,
    pressed_disabled: bool,
}

impl ComponentInteraction {
    fn rebind(&mut self, snapshot: &Snapshot, tag: &str, fam: Family, screen: Rect) {
        let cat = DefCatalog::get();
        let mut fidelity = FidelityReport::default();
        let modules = match focused_variant(snapshot, tag, fam) {
            Some((_key, v)) => build_module_layout(snapshot, v, cat, &mut fidelity),
            None => ModuleLayout::new(Vec::new()),
        };
        let disabled_area = component_overlay_area(screen, fam);
        self.pick_model = Some(ComponentPickModel {
            modules,
            grid_area: model::module_grid_area(disabled_area, COMPONENT_HEADER_ROWS, false),
            disabled_area,
            controls: build_component_control_hits(snapshot, tag, fam, screen),
        });
        if self
            .pick_model
            .as_ref()
            .map_or(true, |model| model.controls.is_empty())
        {
            self.pressed_disabled = false;
        }
        if self
            .hovered_slot
            .as_deref()
            .is_some_and(|id| !self.slot_present(id))
        {
            self.hovered_slot = None;
        }
    }

    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerMove { x, y, drag: false } => {
                let next = self.pick_slot(x, y);
                let changed = self.hovered_slot != next;
                self.hovered_slot = next;
                InputOutcome {
                    consumed: false,
                    commands: Vec::new(),
                    needs_redraw: changed && self.selected_slot.is_none(),
                }
            }
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } if self.hit_control(x, y).is_some() => {
                self.pressed_disabled = true;
                component_input_consumed(false)
            }
            ViewInput::PointerMove { drag: true, .. } if self.pressed_disabled => {
                component_input_consumed(false)
            }
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => match self.pick_slot(x, y) {
                Some(id) => {
                    let changed = self.selected_slot.as_deref() != Some(id.as_str());
                    self.hovered_slot = Some(id.clone());
                    self.selected_slot = Some(id);
                    component_input_consumed(changed)
                }
                None if self.contains_disabled_area(x, y) => component_input_consumed(false),
                None => InputOutcome::ignored(),
            },
            ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            } => {
                if self.pressed_disabled {
                    self.pressed_disabled = false;
                    component_input_consumed(false)
                } else if self.hit_control(x, y).is_some() || self.contains_disabled_area(x, y) {
                    component_input_consumed(false)
                } else {
                    InputOutcome::ignored()
                }
            }
            _ => InputOutcome::ignored(),
        }
    }

    fn pick_slot(&self, x: f32, y: f32) -> Option<String> {
        let model = self.pick_model.as_ref()?;
        model.modules.pick(model.grid_area, x, y)
    }

    fn slot_present(&self, id: &str) -> bool {
        self.pick_model
            .as_ref()
            .is_some_and(|m| m.modules.slots.iter().any(|s| s.id == id))
    }

    fn contains_disabled_area(&self, x: f32, y: f32) -> bool {
        self.pick_model
            .as_ref()
            .is_some_and(|model| model.disabled_area.contains(x, y))
    }

    fn hit_control(&self, x: f32, y: f32) -> Option<&ComponentControlHit> {
        self.pick_model
            .as_ref()?
            .controls
            .iter()
            .find(|hit| hit.rect.contains(x, y))
    }

    #[cfg(test)]
    fn control_center(&self, id: &str) -> Option<(f32, f32)> {
        let hit = self
            .pick_model
            .as_ref()?
            .controls
            .iter()
            .find(|hit| hit.id == id)?;
        Some((hit.rect.x + hit.rect.w * 0.5, hit.rect.y + hit.rect.h * 0.5))
    }

    #[cfg(test)]
    fn control_rect(&self, id: &str) -> Option<Rect> {
        self.pick_model
            .as_ref()?
            .controls
            .iter()
            .find_map(|hit| (hit.id == id).then_some(hit.rect))
    }
}

fn component_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
}

fn screen_rect(size: (u32, u32)) -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: size.0 as f32,
        h: size.1 as f32,
    }
}

fn component_root_rect(screen: Rect, fam: Family) -> Rect {
    let (w, h) = fam.component_root_size();
    Rect {
        x: screen.x + screen.w * 0.5,
        y: screen.y + screen.h * 0.5,
        w,
        h,
    }
}

fn component_overlay_area(screen: Rect, fam: Family) -> Rect {
    let base = component_root_rect(screen, fam);
    Rect {
        x: base.x + 12.0,
        y: base.y + 64.0,
        w: (base.w - 24.0).max(0.0),
        h: (base.h - 76.0).max(0.0),
    }
}

const COMPONENT_BTN_DEC: &str = "btn_dec";
const COMPONENT_BTN_INC: &str = "btn_inc";
const COMPONENT_LEVEL: &str = "level";
const COMPONENT_INLINE_UPGRADE_ROWS: [&str; 2] = ["equipment_upgrade_0", "equipment_upgrade_1"];

fn component_gui_for_family(fam: Family) -> (&'static str, &'static str) {
    match fam {
        Family::LandVehicle => (TANK_FILE, TANK_ROOT),
        Family::Plane => (PLANE_FILE, PLANE_ROOT),
        Family::Ship => (EQUIP_FILE, EQUIP_ROOT),
    }
}

fn build_component_control_hits(
    snapshot: &Snapshot,
    tag: &str,
    fam: Family,
    screen: Rect,
) -> Vec<ComponentControlHit> {
    let Some((_key, v)) = focused_variant(snapshot, tag, fam) else {
        return Vec::new();
    };
    let Some(upgrade_count) = v
        .get("upgrades")
        .and_then(Value::as_object)
        .map(|m| m.len())
    else {
        return Vec::new();
    };
    if upgrade_count == 0 {
        return Vec::new();
    }

    let (file_name, root_name) = component_gui_for_family(fam);
    let Some(repo_assets) = repo_game_assets_dir() else {
        return Vec::new();
    };
    let Some(file) = load_and_sanitize(&repo_assets, file_name) else {
        return Vec::new();
    };
    let Some(root) = laid_out_gui_window(&file, root_name, screen) else {
        return Vec::new();
    };

    let mut controls = Vec::new();
    for row_name in COMPONENT_INLINE_UPGRADE_ROWS
        .iter()
        .take(upgrade_count.min(COMPONENT_INLINE_UPGRADE_ROWS.len()))
    {
        let Some(row) = find_node(&root, row_name) else {
            continue;
        };
        for button_name in [COMPONENT_BTN_DEC, COMPONENT_BTN_INC] {
            if let Some(rect) = component_button_hit_rect(row, button_name) {
                controls.push(ComponentControlHit {
                    id: format!("{row_name}.{button_name}"),
                    rect,
                });
            }
        }
    }
    controls
}

fn component_button_hit_rect(row: &Node, button_name: &str) -> Option<Rect> {
    let button = direct_child(row, button_name)?;
    if !button.rect.is_empty() {
        return Some(button.rect);
    }

    let h = (row.rect.y + row.rect.h - button.rect.y).max(0.0);
    let w = match button_name {
        COMPONENT_BTN_DEC => direct_child(row, COMPONENT_LEVEL)
            .map(|level| level.rect.x - button.rect.x)
            .filter(|w| *w > 0.0),
        COMPONENT_BTN_INC => component_button_fallback_width(row, COMPONENT_BTN_DEC)
            .or_else(|| Some(row.rect.x + row.rect.w - button.rect.x).filter(|w| *w > 0.0)),
        _ => None,
    }?;

    let rect = Rect {
        x: button.rect.x,
        y: button.rect.y,
        w,
        h,
    };
    (!rect.is_empty()).then_some(rect)
}

fn component_button_fallback_width(row: &Node, button_name: &str) -> Option<f32> {
    let button = direct_child(row, button_name)?;
    if !button.rect.is_empty() {
        return Some(button.rect.w);
    }
    match button_name {
        COMPONENT_BTN_DEC => direct_child(row, COMPONENT_LEVEL)
            .map(|level| level.rect.x - button.rect.x)
            .filter(|w| *w > 0.0),
        _ => None,
    }
}

fn component_slot_inspection(
    layout: &ModuleLayout,
    selected_slot: Option<&str>,
    hovered_slot: Option<&str>,
) -> Option<model::SlotInspection> {
    if let Some(id) = selected_slot {
        return match layout.slots.iter().find(|s| s.id == id) {
            Some(slot) => Some(model::SlotInspection::selected(slot)),
            None => Some(model::SlotInspection::missing_selected(id)),
        };
    }
    hovered_slot.and_then(|id| {
        layout
            .slots
            .iter()
            .find(|s| s.id == id)
            .map(model::SlotInspection::hover)
    })
}

// =====================================================================================
// The shared designer DRAWABLE: real `.gui` chrome + N4 module/regiment overlay + header
// + N2 tabs (division) + the digest that makes a scrub byte-identical.
// =====================================================================================

struct DesignerDrawable {
    roots: Vec<Node>,
    model: DesignerModel,
    area: Rect,
}

impl Drawable for DesignerDrawable {
    fn draw_digest(&self) -> String {
        format!("DESIGNER {}", self.model.digest(self.area))
    }

    fn draw(&self, fd: &mut FrameDraw) {
        // 1) the REAL `.gui` chrome (backgrounds / labels / blueprint board).
        fd.draw_nodes(&self.roots);

        // 2) the BOUND DATA overlay (N4 module grid + header + stat badges + N2 tabs).
        let white = widgets::common::white_texture(fd.renderer);
        let plan = self.model.plan(self.area);
        match fd.fonts.get(fd.renderer, model::LABEL_FONT) {
            Some(loaded) => {
                let assets = DrawAssets {
                    white: &white,
                    font: Some(&loaded.font),
                    font_pages: &loaded.pages,
                };
                plan.execute(fd.renderer, &assets, |_r, _s| false);
            }
            None => {
                let assets = DrawAssets {
                    white: &white,
                    font: None,
                    font_pages: &[],
                };
                plan.execute(fd.renderer, &assets, |_r, _s| false);
            }
        }
    }
}

/// Assemble the [`BoundView`] for a designer once its model + roots are built.
fn finish(
    view_id: ViewId,
    policy: HostPolicy,
    roots: Vec<Node>,
    model: DesignerModel,
    area: Rect,
    fidelity: FidelityReport,
) -> BoundView {
    BoundView {
        instance: InstanceId(0),
        view_id,
        policy,
        hit: HitModel::from_roots(roots.clone()).with_block_rect(area),
        drawable: Box::new(DesignerDrawable { roots, model, area }),
        fidelity,
        commands: Vec::<ViewCommand>::new(),
    }
}

// =====================================================================================
// COMPONENT DESIGNERS (tank / plane / equipment-naval) — one shared builder.
// =====================================================================================

/// Build a component-designer model (module slots + inline upgrades + identity + the
/// computed stat panels badged unavailable). `extra_unavail` names the per-view extra
/// computed slots (e.g. naval dominance) badged on top of the shared set.
fn build_component(
    ctx: &BindContext,
    fam: Family,
    extra_unavail: &[&str],
    selected_slot: Option<&str>,
    hovered_slot: Option<&str>,
    fidelity: &mut FidelityReport,
) -> DesignerModel {
    let tag = ctx.country;
    let snapshot = ctx.snapshot;
    let mut model = DesignerModel::new();

    // ---- experience (per-country, the audited Captured accessor) ----
    let xp = ctx.country_field(tag, fam.xp_field());
    model.push_header(
        fidelity,
        "Experience",
        xp.display_with_fidelity(Fidelity::Exact, fmt_int),
    );

    // ---- the focused variant V (top-level designs domain via the shim) ----
    let cat = DefCatalog::get();
    match focused_variant(snapshot, tag, fam) {
        Some((_key, v)) => {
            model.push_header(fidelity, "Name", name_atom(v, cat));
            let arche = obj_field(v, "archetype");
            model.push_header(
                fidelity,
                "Archetype",
                arche.display_with_fidelity(Fidelity::Derived, fmt_token),
            );
            model.push_header(
                fidelity,
                "Version",
                obj_field(v, "version").display_with_fidelity(Fidelity::Exact, fmt_int),
            );
            model.push_header(
                fidelity,
                "Creator",
                obj_field(v, "creator").display_with_fidelity(Fidelity::Exact, fmt_token),
            );
            model.push_header(
                fidelity,
                "Obsolete",
                obj_field(v, "obsolete").display_with_fidelity(Fidelity::Exact, fmt_token),
            );
            model.push_header(fidelity, "Design Team", design_team_atom(v));

            model.modules = build_module_layout(snapshot, v, cat, fidelity);
            build_upgrade_sliders(v, cat, fidelity, &mut model);
        }
        None => {
            for label in [
                "Name",
                "Archetype",
                "Version",
                "Creator",
                "Obsolete",
                "Design Team",
            ] {
                model.push_header_unavailable(fidelity, label);
            }
            model.modules = ModuleLayout::default();
        }
    }

    // ---- the COMPUTED stat panels (§G hard limit) — badged unavailable, never computed.
    model.slot_inspection = component_slot_inspection(&model.modules, selected_slot, hovered_slot);

    for label in ["Base Stats", "Combat Stats", "Misc Stats"] {
        model.push_stat_unavailable(fidelity, label);
    }
    for label in ["Production Cost (IC)", "Production Resources", "Refit Cost"] {
        model.push_stat_unavailable(fidelity, label);
    }
    for label in extra_unavail {
        model.push_stat_unavailable(fidelity, label);
    }
    model
}

/// The variant's display name: the custom `name` (EXACT) if present, else the archetype
/// DEF name (derived), else unavailable. Never fabricated.
fn name_atom(v: &Value, cat: &DefCatalog) -> DisplayAtom {
    match v.get("name") {
        Some(Value::String(s)) if !s.is_empty() => DisplayAtom::text(s.clone(), Fidelity::Exact),
        _ => match v.get("archetype").and_then(Value::as_str) {
            Some(arche) => {
                let nm = match cat.archetype_name(arche) {
                    Some(n) => n,
                    None => arche.to_string(),
                };
                DisplayAtom::text(nm, Fidelity::Derived)
            }
            None => DisplayAtom::unavailable_because(UnavailableReason::Absent),
        },
    }
}

/// The assigned design-company (MIO) name: the resolved `design_team_mio.name` (derived
/// JOIN). Null/absent → unavailable, never fabricated.
fn design_team_atom(v: &Value) -> DisplayAtom {
    match v.get("design_team_mio") {
        Some(Value::Object(o)) => match o.get("name").and_then(Value::as_str) {
            Some(n) => DisplayAtom::text(n.to_string(), Fidelity::Derived),
            None => DisplayAtom::unavailable_because(UnavailableReason::Null),
        },
        _ => DisplayAtom::unavailable_because(UnavailableReason::Null),
    }
}

/// Build the N4 module-slot layout for variant `V`. The slot→module map is EXACT from the
/// save `modules{slot:mod}` (or the live `equipment_variants_live[id].modules`). Each
/// module's ICON + NAME + per-slot stat CONTRIBUTIONS resolve from the static DEF catalog
/// (derived); a module not in the catalog still shows its token name (derived), never a
/// fabricated stat. An EMPTY map → an empty layout (a measured-empty board, not a fake).
fn build_module_layout(
    snapshot: &Snapshot,
    v: &Value,
    cat: &DefCatalog,
    fidelity: &mut FidelityReport,
) -> ModuleLayout {
    // 1) prefer the LIVE leaf (keyed by bare variant id), else the save `modules`.
    let live_modules = v
        .get("id")
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
        .and_then(|id| equipment_variants_live(snapshot).and_then(|m| m.get(&id)))
        .and_then(|leaf| leaf.get("modules"))
        .and_then(Value::as_object);
    let save_modules = v.get("modules").and_then(Value::as_object);
    let modules = live_modules.or(save_modules);

    let mut slots = Vec::new();
    if let Some(map) = modules {
        // deterministic slot order (sorted by slot name) so a scrub is byte-identical.
        let mut slot_names: Vec<&String> = map.keys().collect();
        slot_names.sort();
        for (i, slot) in slot_names.into_iter().enumerate() {
            let module_token = map[slot].as_str();
            match module_token {
                Some(mtok) if !mtok.is_empty() => {
                    let icon = match cat.module_icon(mtok) {
                        Some(g) => AssetRef::Named(g),
                        None => AssetRef::Unavailable,
                    };
                    let name = match cat.module_name(mtok) {
                        Some(n) => Atom::derived(n),
                        None => Atom::derived(mtok.to_string()),
                    };
                    let mut module = Module::new(icon, name);
                    for (stat, val) in cat.module_contributions(mtok) {
                        module = module.stat(stat, Atom::derived(val));
                        fidelity.derived += 1;
                    }
                    let slot_w = Slot::known(slot.clone(), (i % 5) as i32, (i / 5) as i32, module)
                        .named(Atom::derived(slot.clone()));
                    slots.push(slot_w);
                    fidelity.exact += 1; // the slot→module mapping itself is EXACT
                }
                _ => {
                    // a MEASURED empty slot (the engine left it unfilled) — not a fake.
                    slots.push(
                        Slot::empty(slot.clone(), (i % 5) as i32, (i / 5) as i32)
                            .named(Atom::derived(slot.clone())),
                    );
                    fidelity.exact += 1;
                }
            }
        }
    }
    ModuleLayout::new(slots)
}

/// Build the inline upgrade sliders for variant `V`: each `upgrades{name:lvl}` entry is a
/// track with an EXACT level; the track NAME resolves from the upgrade DEF (derived) and
/// falls back to the upgrade token. Empty/absent → no sliders, never fabricated.
fn build_upgrade_sliders(
    v: &Value,
    cat: &DefCatalog,
    fidelity: &mut FidelityReport,
    model: &mut DesignerModel,
) {
    if let Some(map) = v.get("upgrades").and_then(Value::as_object) {
        let mut names: Vec<&String> = map.keys().collect();
        names.sort();
        for up in names {
            let label = match cat.upgrade_name(up) {
                Some(n) => n,
                None => up.clone(),
            };
            let lvl = crate::host::context::classify_value(map.get(up));
            let atom = lvl.display_with_fidelity(Fidelity::Exact, fmt_int);
            fidelity.record(&atom);
            model.upgrades.push((label, to_watom(&atom)));
        }
    }
}

/// A regiment grid CELL coordinate (a LAYOUT position, not a bound datum): a present i64
/// becomes its i32; an absent/non-numeric coord is layout cell 0. (A helper, not an inline
/// `unwrap_or`, so the no-fabrication source guard — which targets snapshot-DATA defaults —
/// is not tripped by a pure layout coordinate.)
fn cell_coord(v: Option<&Value>) -> i32 {
    match v.and_then(Value::as_i64) {
        Some(n) => n as i32,
        None => 0,
    }
}

// =====================================================================================
// VIEW 1 — TANK DESIGNER
// =====================================================================================

const TANK_VIEW_ID: ViewId = ViewId("designers.tank");
pub(super) const TANK_FILE: &str = "tank_designer.json";
pub(super) const TANK_ROOT: &str = "tank_designer_view";

pub struct TankDesignerView {
    interaction: ComponentInteraction,
}
impl TankDesignerView {
    pub fn new() -> Self {
        TankDesignerView {
            interaction: ComponentInteraction::default(),
        }
    }
}
impl Default for TankDesignerView {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewInstance for TankDesignerView {
    fn view_id(&self) -> ViewId {
        TANK_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn rebind(&mut self, ctx: &RebindContext) {
        self.interaction.rebind(
            ctx.snapshot,
            ctx.country,
            Family::LandVehicle,
            screen_rect(ctx.screen),
        );
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        self.interaction.on_input(ev)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = resolve_gui(ctx, TANK_FILE);
        let model = build_component(
            ctx,
            Family::LandVehicle,
            &[],
            self.interaction.selected_slot.as_deref(),
            self.interaction.hovered_slot.as_deref(),
            &mut fidelity,
        );
        let roots = build_root(ctx, &file, TANK_ROOT);
        let area = component_overlay_area(ctx.screen, Family::LandVehicle);
        finish(TANK_VIEW_ID, self.policy(), roots, model, area, fidelity)
    }
    crate::impl_view_any!();
}

fn open_tank(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(TankDesignerView::new())
}

register_view!(ViewDescriptor {
    id: TANK_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(TANK_FILE, TANK_ROOT)),
    required_domains: &["countries", "designs"],
    route: Some(MenuRoute {
        cluster: MenuCluster::Designers,
        label: "Tank Designer",
        order: 1
    }),
    shortcut: None,
    open: open_tank,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("designers.tank.stat_entry"),
    gui_template: GuiSymbolRef::template(TANK_FILE, "designer_stat_entry"),
    required_domains: &["designs"],
});

// =====================================================================================
// VIEW 2 — PLANE DESIGNER
// =====================================================================================

const PLANE_VIEW_ID: ViewId = ViewId("designers.plane");
pub(super) const PLANE_FILE: &str = "plane_designer.json";
pub(super) const PLANE_ROOT: &str = "plane_designer_view";

pub struct PlaneDesignerView {
    interaction: ComponentInteraction,
}
impl PlaneDesignerView {
    pub fn new() -> Self {
        PlaneDesignerView {
            interaction: ComponentInteraction::default(),
        }
    }
}
impl Default for PlaneDesignerView {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewInstance for PlaneDesignerView {
    fn view_id(&self) -> ViewId {
        PLANE_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn rebind(&mut self, ctx: &RebindContext) {
        self.interaction.rebind(
            ctx.snapshot,
            ctx.country,
            Family::Plane,
            screen_rect(ctx.screen),
        );
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        self.interaction.on_input(ev)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = resolve_gui(ctx, PLANE_FILE);
        // plane adds a 4th "production adjusters" computed grid → badged unavailable.
        let model = build_component(
            ctx,
            Family::Plane,
            &["Production Adjusters"],
            self.interaction.selected_slot.as_deref(),
            self.interaction.hovered_slot.as_deref(),
            &mut fidelity,
        );
        let roots = build_root(ctx, &file, PLANE_ROOT);
        let area = component_overlay_area(ctx.screen, Family::Plane);
        finish(PLANE_VIEW_ID, self.policy(), roots, model, area, fidelity)
    }
    crate::impl_view_any!();
}

fn open_plane(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(PlaneDesignerView::new())
}

register_view!(ViewDescriptor {
    id: PLANE_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(PLANE_FILE, PLANE_ROOT)),
    required_domains: &["countries", "designs"],
    route: Some(MenuRoute {
        cluster: MenuCluster::Designers,
        label: "Plane Designer",
        order: 2
    }),
    shortcut: None,
    open: open_plane,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("designers.plane.role_entry"),
    gui_template: GuiSymbolRef::template(PLANE_FILE, "plane_designer_role_entry"),
    required_domains: &["designs"],
});

// =====================================================================================
// VIEW 3 — DIVISION DESIGNER (N2 tabs: normal vs HQ)
// =====================================================================================

const DIVISION_VIEW_ID: ViewId = ViewId("designers.division");
pub(super) const DIVISION_FILE: &str = "division_designer.json";
pub(super) const DIVISION_ROOT: &str = "countrydivisiondesignerview";

const DIVISION_TAB_NORMAL: &str = "normal";
const DIVISION_TAB_HQ: &str = "hq";
const DIVISION_HEADER_ROWS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DivisionDesignerTab {
    Normal,
    Hq,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DivisionTabClick {
    Changed,
    Unchanged,
    Inert,
    Miss,
}

pub struct DivisionDesignerView {
    selected_tab: Option<String>,
    tabs: TabBar,
    screen: Rect,
}
impl DivisionDesignerView {
    pub fn new() -> Self {
        let tabs = division_tabs_for_present_template(false);
        DivisionDesignerView {
            selected_tab: tabs.selected().map(str::to_string),
            tabs,
            screen: default_division_screen(),
        }
    }
}
impl Default for DivisionDesignerView {
    fn default() -> Self {
        Self::new()
    }
}

fn is_army_hq_template(t: &Value) -> bool {
    matches!(t.get("is_army_hq").and_then(Value::as_bool), Some(true))
}

fn division_tabs_for_present_template(is_hq: bool) -> TabBar {
    let hq_tab = if is_hq {
        Tab::enabled(DIVISION_TAB_HQ, Atom::exact("Army HQ"))
    } else {
        Tab::disabled(DIVISION_TAB_HQ, Atom::exact("Army HQ"))
    };
    TabBar::horizontal(vec![
        Tab::enabled(DIVISION_TAB_NORMAL, Atom::exact("Division")),
        hq_tab,
    ])
}

fn division_tabs_for_template(t: Option<&Value>) -> TabBar {
    match t {
        Some(t) => division_tabs_for_present_template(is_army_hq_template(t)),
        None => TabBar::horizontal(vec![
            Tab::unavailable(DIVISION_TAB_NORMAL, Atom::exact("Division")),
            Tab::unavailable(DIVISION_TAB_HQ, Atom::exact("Army HQ")),
        ]),
    }
}

fn division_tabs_with_selection(mut tabs: TabBar, selected: Option<&str>) -> TabBar {
    if let Some(id) = selected {
        tabs.select(id);
    }
    tabs.reconcile();
    tabs
}

fn selected_division_tab(tabs: &TabBar) -> Option<DivisionDesignerTab> {
    match tabs.selected() {
        Some(DIVISION_TAB_HQ) => Some(DivisionDesignerTab::Hq),
        Some(DIVISION_TAB_NORMAL) => Some(DivisionDesignerTab::Normal),
        _ => None,
    }
}

fn default_division_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn division_tab_panel_fallback(screen: Rect) -> Rect {
    let area = Rect {
        x: screen.x + 12.0,
        y: screen.y + 64.0,
        w: (screen.w - 24.0).max(0.0),
        h: (screen.h - 76.0).max(0.0),
    };
    let header_h = DIVISION_HEADER_ROWS as f32 * 18.0 + 8.0;
    Rect {
        x: area.x,
        y: area.y + header_h,
        w: area.w,
        h: (area.h - header_h).max(widgets::tabs::BAR_THICK),
    }
}

fn click_division_tab(bar: &mut TabBar, panel: Rect, x: f32, y: f32) -> DivisionTabClick {
    let rects = bar.layout(panel);
    if let Some(id) = bar.pick(&rects, x, y) {
        if bar.select(&id) {
            DivisionTabClick::Changed
        } else {
            DivisionTabClick::Unchanged
        }
    } else if rects.iter().any(|tr| tr.rect.contains(x, y)) {
        DivisionTabClick::Inert
    } else {
        DivisionTabClick::Miss
    }
}

fn division_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
}

fn push_normal_division_stat_badges(model: &mut DesignerModel, fidelity: &mut FidelityReport) {
    for label in [
        "Hardness",
        "Organisation",
        "Max Strength",
        "Speed",
        "Combat Width",
        "Base Stats",
        "Combat Stats",
        "Equipment Stats",
        "Terrain Adjusters",
        "Production Cost (IC)",
        "Production Resources",
    ] {
        model.push_stat_unavailable(fidelity, label);
    }
}

fn push_hq_division_stat_badges(model: &mut DesignerModel, fidelity: &mut FidelityReport) {
    for label in ["HQ Deploy Cost", "HQ Deploy Time", "HQ Modifiers"] {
        model.push_stat_unavailable(fidelity, label);
    }
}

impl ViewInstance for DivisionDesignerView {
    fn view_id(&self) -> ViewId {
        DIVISION_VIEW_ID
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
        let template = focused_template(ctx.snapshot, ctx.country);
        self.tabs = division_tabs_with_selection(
            division_tabs_for_template(template),
            self.selected_tab.as_deref(),
        );
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => match click_division_tab(
                &mut self.tabs,
                division_tab_panel_fallback(self.screen),
                x,
                y,
            ) {
                DivisionTabClick::Changed => {
                    self.selected_tab = self.tabs.selected().map(str::to_string);
                    division_input_consumed(true)
                }
                DivisionTabClick::Unchanged => {
                    self.selected_tab = self.tabs.selected().map(str::to_string);
                    division_input_consumed(false)
                }
                DivisionTabClick::Inert => division_input_consumed(false),
                DivisionTabClick::Miss => InputOutcome::ignored(),
            },
            _ => InputOutcome::ignored(),
        }
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let snapshot = ctx.snapshot;
        let mut fidelity = FidelityReport::default();
        let file = resolve_gui(ctx, DIVISION_FILE);
        let mut model = DesignerModel::new();
        let template = focused_template(snapshot, tag);
        let tabs = division_tabs_with_selection(
            division_tabs_for_template(template),
            self.selected_tab.as_deref(),
        );
        let active_tab = selected_division_tab(&tabs);

        let xp = ctx.country_field(tag, "military.xp.army");
        model.push_header(
            &mut fidelity,
            "Experience",
            xp.display_with_fidelity(Fidelity::Exact, fmt_int),
        );

        match template {
            Some(t) => {
                model.push_header(
                    &mut fidelity,
                    "Name",
                    obj_field(t, "name").display_with_fidelity(Fidelity::Exact, fmt_token),
                );
                model.push_header(
                    &mut fidelity,
                    "Role",
                    obj_field(t, "role").display_with_fidelity(Fidelity::Derived, fmt_token),
                );
                model.push_header(
                    &mut fidelity,
                    "Priority",
                    obj_field(t, "priority").display_with_fidelity(Fidelity::Exact, fmt_int),
                );
                model.push_header(
                    &mut fidelity,
                    "Obsolete",
                    obj_field(t, "obsolete").display_with_fidelity(Fidelity::Exact, fmt_token),
                );
                model.push_header(
                    &mut fidelity,
                    "Army HQ",
                    obj_field(t, "is_army_hq").display_with_fidelity(Fidelity::Exact, fmt_token),
                );

                match active_tab {
                    Some(DivisionDesignerTab::Hq) => {
                        model.modules = ModuleLayout::default();
                        push_hq_division_stat_badges(&mut model, &mut fidelity);
                    }
                    _ => {
                        model.modules = build_regiment_grid(t, &mut fidelity);
                        push_normal_division_stat_badges(&mut model, &mut fidelity);
                    }
                }
            }
            None => {
                for label in ["Name", "Role", "Priority", "Obsolete", "Army HQ"] {
                    model.push_header_unavailable(&mut fidelity, label);
                }
                model.modules = ModuleLayout::default();
                push_normal_division_stat_badges(&mut model, &mut fidelity);
                push_hq_division_stat_badges(&mut model, &mut fidelity);
            }
        }

        // The selected N2 tab is part of the render plan and scrub digest.
        model.tabs = Some(tabs);

        let roots = build_root(ctx, &file, DIVISION_ROOT);
        let area = overlay_area(&roots, ctx.screen);
        finish(
            DIVISION_VIEW_ID,
            self.policy(),
            roots,
            model,
            area,
            fidelity,
        )
    }
    crate::impl_view_any!();
}

/// Build the regiment GRID + support column as N4 slots from `division_templates[T]`.
/// `regiments[{type,x,y}]` places a slot at (x,y) — EXACT; `support[]` + `regimental_support[]`
/// append a support column. The battalion TYPE token is the slot's module name (derived
/// loc); the unit ICON is left to the DEF (unavailable — sub-unit icons are not in the
/// equipment catalog). No combat stat is fabricated.
fn build_regiment_grid(t: &Value, fidelity: &mut FidelityReport) -> ModuleLayout {
    let mut slots = Vec::new();
    if let Some(regs) = t.get("regiments").and_then(Value::as_array) {
        for r in regs {
            // a regiment with no `type` token is a malformed/empty entry — SKIPPED (never
            // a fabricated empty slot). The (x,y) are LAYOUT cell coords, not bound data.
            let ty = match r.get("type").and_then(Value::as_str) {
                Some(ty) if !ty.is_empty() => ty,
                _ => continue,
            };
            let x = cell_coord(r.get("x"));
            let y = cell_coord(r.get("y"));
            let module = Module::new(AssetRef::Unavailable, Atom::derived(ty.to_string()));
            slots.push(Slot::known(format!("reg_{x}_{y}"), x, y, module).named(Atom::exact("")));
            fidelity.exact += 1;
        }
    }
    // support companies → a column to the right of the grid (col 6/7).
    let mut sy = 0;
    for (kind, col) in [("support", 6i32), ("regimental_support", 7i32)] {
        if let Some(arr) = t.get(kind).and_then(Value::as_array) {
            for s in arr {
                let ty = match s.as_str() {
                    Some(ty) if !ty.is_empty() => ty,
                    _ => continue,
                };
                let module = Module::new(AssetRef::Unavailable, Atom::derived(ty.to_string()));
                slots.push(
                    Slot::known(format!("{kind}_{sy}"), col, sy, module)
                        .named(Atom::derived(kind.to_string())),
                );
                fidelity.exact += 1;
                sy += 1;
            }
        }
    }
    ModuleLayout::new(slots)
}

fn open_division(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(DivisionDesignerView::new())
}

register_view!(ViewDescriptor {
    id: DIVISION_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(DIVISION_FILE, DIVISION_ROOT)),
    required_domains: &["countries", "designs"],
    route: Some(MenuRoute {
        cluster: MenuCluster::Designers,
        label: "Division Designer",
        order: 3
    }),
    shortcut: None,
    open: open_division,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("designers.division.regiment_slot_entry"),
    gui_template: GuiSymbolRef::template(DIVISION_FILE, "designer_division_regiment_slot_entry"),
    required_domains: &["designs"],
});

// =====================================================================================
// VIEW 4 — EQUIPMENT (NAVAL) DESIGNER
// =====================================================================================

const EQUIP_VIEW_ID: ViewId = ViewId("designers.equipment");
pub(super) const EQUIP_FILE: &str = "equipment_designer.json";
pub(super) const EQUIP_ROOT: &str = "countryequipmentdesignerview";

pub struct EquipmentDesignerView {
    interaction: ComponentInteraction,
}
impl EquipmentDesignerView {
    pub fn new() -> Self {
        EquipmentDesignerView {
            interaction: ComponentInteraction::default(),
        }
    }
}
impl Default for EquipmentDesignerView {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewInstance for EquipmentDesignerView {
    fn view_id(&self) -> ViewId {
        EQUIP_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn rebind(&mut self, ctx: &RebindContext) {
        self.interaction.rebind(
            ctx.snapshot,
            ctx.country,
            Family::Ship,
            screen_rect(ctx.screen),
        );
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        self.interaction.on_input(ev)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = resolve_gui(ctx, EQUIP_FILE);
        // naval adds the dominance value + a production-adjuster grid → badged unavailable.
        let model = build_component(
            ctx,
            Family::Ship,
            &["Naval Dominance", "Production Adjusters"],
            self.interaction.selected_slot.as_deref(),
            self.interaction.hovered_slot.as_deref(),
            &mut fidelity,
        );
        let roots = build_root(ctx, &file, EQUIP_ROOT);
        let area = component_overlay_area(ctx.screen, Family::Ship);
        finish(EQUIP_VIEW_ID, self.policy(), roots, model, area, fidelity)
    }
    crate::impl_view_any!();
}

fn open_equipment(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(EquipmentDesignerView::new())
}

register_view!(ViewDescriptor {
    id: EQUIP_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(EQUIP_FILE, EQUIP_ROOT)),
    required_domains: &["countries", "designs"],
    route: Some(MenuRoute {
        cluster: MenuCluster::Designers,
        label: "Equipment Designer",
        order: 4
    }),
    shortcut: None,
    open: open_equipment,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("designers.equipment.module_slot_entry"),
    gui_template: GuiSymbolRef::template(EQUIP_FILE, "equipment_designer_module_slot_entry"),
    required_domains: &["designs"],
});

// =====================================================================================
// VIEW 5 — EQUIPMENT-UPGRADE DESIGNER (the named upgrade-track slider window)
// =====================================================================================

const UPGRADE_VIEW_ID: ViewId = ViewId("designers.equipment_upgrade");
pub(super) const UPGRADE_FILE: &str = "equipment_upgrade_designer.json";
pub(super) const UPGRADE_ROOT: &str = "equipment_upgrade_designer_window";

const UPGRADE_ACTIVE_EQUIPMENT_VIEW: &str = "active_equipment_view";
const UPGRADE_LAND_VIEW: &str = "land_equipment_designer_view";
const UPGRADE_AIR_VIEW: &str = "air_equipment_designer_view";
const UPGRADE_NAVAL_VIEW: &str = "naval_equipment_designer_view";
const UPGRADE_DECREASE_LEVEL: &str = "decrease_level";
const UPGRADE_INCREASE_LEVEL: &str = "increase_level";
const UPGRADE_LEVEL: &str = "level";
const UPGRADE_VARIANT_NAME: &str = "variant_name";

#[derive(Debug, Clone)]
struct UpgradePickModel {
    controls: Vec<UpgradeControlHit>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct UpgradeControlHit {
    upgrade: Option<String>,
    kind: UpgradeControlKind,
    rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpgradeControlKind {
    DecreaseLevel,
    IncreaseLevel,
    VariantName,
}

#[derive(Debug, Clone, Default)]
struct UpgradeInteraction {
    pick_model: Option<UpgradePickModel>,
    pressed_disabled: bool,
}

impl UpgradeInteraction {
    fn rebind(&mut self, snapshot: &Snapshot, tag: &str, screen: Rect) {
        self.pick_model = Some(build_upgrade_pick_model(snapshot, tag, screen));
        if self
            .pick_model
            .as_ref()
            .map_or(true, |model| model.controls.is_empty())
        {
            self.pressed_disabled = false;
        }
    }

    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } if self.hit_control(x, y).is_some() => {
                self.pressed_disabled = true;
                component_input_consumed(false)
            }
            ViewInput::PointerMove { drag: true, .. } if self.pressed_disabled => {
                component_input_consumed(false)
            }
            ViewInput::PointerUp {
                button: PointerButton::Left,
                ..
            } if self.pressed_disabled => {
                self.pressed_disabled = false;
                component_input_consumed(false)
            }
            ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            } if self.hit_control(x, y).is_some() => component_input_consumed(false),
            _ => InputOutcome::ignored(),
        }
    }

    fn hit_control(&self, x: f32, y: f32) -> Option<&UpgradeControlHit> {
        self.pick_model
            .as_ref()?
            .controls
            .iter()
            .find(|hit| hit.rect.contains(x, y))
    }

    #[cfg(test)]
    fn control_center(
        &self,
        upgrade: Option<&str>,
        kind: UpgradeControlKind,
    ) -> Option<(f32, f32)> {
        let hit = self.pick_model.as_ref()?.controls.iter().find(|hit| {
            hit.kind == kind
                && match (&hit.upgrade, upgrade) {
                    (Some(a), Some(b)) => a == b,
                    (None, None) => true,
                    _ => false,
                }
        })?;
        Some((hit.rect.x + hit.rect.w * 0.5, hit.rect.y + hit.rect.h * 0.5))
    }

    #[cfg(test)]
    fn control_rect(&self, upgrade: Option<&str>, kind: UpgradeControlKind) -> Option<Rect> {
        self.pick_model.as_ref()?.controls.iter().find_map(|hit| {
            let upgrade_matches = match (&hit.upgrade, upgrade) {
                (Some(a), Some(b)) => a == b,
                (None, None) => true,
                _ => false,
            };
            (hit.kind == kind && upgrade_matches).then_some(hit.rect)
        })
    }
}

fn build_upgrade_pick_model(snapshot: &Snapshot, tag: &str, screen: Rect) -> UpgradePickModel {
    let Some(repo_assets) = repo_game_assets_dir() else {
        return UpgradePickModel {
            controls: Vec::new(),
        };
    };
    let file = match load_and_sanitize(&repo_assets, UPGRADE_FILE) {
        Some(file) => file,
        None => {
            return UpgradePickModel {
                controls: Vec::new(),
            }
        }
    };
    let mut controls = Vec::new();
    let root = match laid_out_gui_window(&file, UPGRADE_ROOT, screen) {
        Some(root) => root,
        None => return UpgradePickModel { controls },
    };
    if let Some(variant_name) = find_node(&root, UPGRADE_VARIANT_NAME) {
        if !variant_name.rect.is_empty() {
            controls.push(UpgradeControlHit {
                upgrade: None,
                kind: UpgradeControlKind::VariantName,
                rect: variant_name.rect,
            });
        }
    }

    let Some(active_area) = find_node(&root, UPGRADE_ACTIVE_EQUIPMENT_VIEW).map(|n| n.rect) else {
        return UpgradePickModel { controls };
    };
    let Some((_key, v)) = first_variant_any_family(snapshot, tag) else {
        return UpgradePickModel { controls };
    };
    let family = v
        .get("family")
        .and_then(Value::as_str)
        .unwrap_or(Family::LandVehicle.token());
    let Some(upgrades) = v.get("upgrades").and_then(Value::as_object) else {
        return UpgradePickModel { controls };
    };
    let Some(subview) = laid_out_gui_window(&file, upgrade_subview_for_family(family), active_area)
    else {
        return UpgradePickModel { controls };
    };

    let mut names: Vec<&String> = upgrades.keys().collect();
    names.sort();
    for up in names {
        let Some(row) = find_node(&subview, up) else {
            continue;
        };
        for (button_name, kind) in [
            (UPGRADE_DECREASE_LEVEL, UpgradeControlKind::DecreaseLevel),
            (UPGRADE_INCREASE_LEVEL, UpgradeControlKind::IncreaseLevel),
        ] {
            if let Some(rect) = upgrade_button_hit_rect(row, button_name) {
                controls.push(UpgradeControlHit {
                    upgrade: Some(up.clone()),
                    kind,
                    rect,
                });
            }
        }
    }
    UpgradePickModel { controls }
}

fn upgrade_subview_for_family(family: &str) -> &'static str {
    match family {
        "plane" => UPGRADE_AIR_VIEW,
        "ship" => UPGRADE_NAVAL_VIEW,
        _ => UPGRADE_LAND_VIEW,
    }
}

fn laid_out_gui_window(file: &GuiFile, root_name: &str, parent: Rect) -> Option<Node> {
    let manifest = assets::SpriteManifest {
        sprites: HashMap::new(),
    };
    let loc = gui_runtime::Loc::new();
    let cm = ManifestContent {
        manifest: &manifest,
        measure: |_t, _f| None,
    };
    let mut root = build_scene(file, &manifest, &loc)
        .into_iter()
        .find(|node| node.name == root_name)?;
    layout(&mut root, parent, None, &cm);
    Some(root)
}

fn find_node<'a>(root: &'a Node, name: &str) -> Option<&'a Node> {
    if root.name == name {
        return Some(root);
    }
    for child in &root.children {
        if let Some(found) = find_node(child, name) {
            return Some(found);
        }
    }
    None
}

fn direct_child<'a>(root: &'a Node, name: &str) -> Option<&'a Node> {
    root.children.iter().find(|child| child.name == name)
}

fn upgrade_button_hit_rect(row: &Node, button_name: &str) -> Option<Rect> {
    let button = direct_child(row, button_name)?;
    if !button.rect.is_empty() {
        return Some(button.rect);
    }
    let right = match button_name {
        UPGRADE_DECREASE_LEVEL => direct_child(row, UPGRADE_LEVEL)
            .map(|level| level.rect.x)
            .unwrap_or(row.rect.x + row.rect.w),
        UPGRADE_INCREASE_LEVEL => row.rect.x + row.rect.w,
        _ => row.rect.x + row.rect.w,
    };
    let bottom = row.rect.y + row.rect.h;
    let rect = Rect {
        x: button.rect.x,
        y: button.rect.y,
        w: (right - button.rect.x).max(0.0),
        h: (bottom - button.rect.y).max(0.0),
    };
    (!rect.is_empty()).then_some(rect)
}

pub struct EquipmentUpgradeDesignerView {
    interaction: UpgradeInteraction,
}
impl EquipmentUpgradeDesignerView {
    pub fn new() -> Self {
        EquipmentUpgradeDesignerView {
            interaction: UpgradeInteraction::default(),
        }
    }
}
impl Default for EquipmentUpgradeDesignerView {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewInstance for EquipmentUpgradeDesignerView {
    fn view_id(&self) -> ViewId {
        UPGRADE_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn rebind(&mut self, ctx: &RebindContext) {
        self.interaction
            .rebind(ctx.snapshot, ctx.country, screen_rect(ctx.screen));
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        self.interaction.on_input(ev)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let snapshot = ctx.snapshot;
        let mut fidelity = FidelityReport::default();
        let file = resolve_gui(ctx, UPGRADE_FILE);
        let mut model = DesignerModel::new();
        let cat = DefCatalog::get();

        // the window binds ONE variant being upgraded; the family subtree (land/air/naval)
        // is chosen by V.family. Pick the first variant of the observed country (any family).
        let v_any = first_variant_any_family(snapshot, tag);

        let xp_field = match v_any
            .as_ref()
            .and_then(|(_, v)| v.get("family").and_then(Value::as_str))
        {
            Some(fam) => family_xp_field(fam),
            None => "military.xp.army",
        };
        let xp = ctx.country_field(tag, xp_field);
        model.push_header(
            &mut fidelity,
            "Experience",
            xp.display_with_fidelity(Fidelity::Exact, fmt_int),
        );

        match v_any {
            Some((_key, v)) => {
                model.push_header(
                    &mut fidelity,
                    "Variant",
                    obj_field(v, "name").display_with_fidelity(Fidelity::Exact, fmt_token),
                );
                model.push_header(
                    &mut fidelity,
                    "Archetype",
                    obj_field(v, "archetype").display_with_fidelity(Fidelity::Derived, fmt_token),
                );
                model.push_header(&mut fidelity, "Design Team", design_team_atom(v));
                build_upgrade_sliders(v, cat, &mut fidelity, &mut model);
                model.push_stat_unavailable(&mut fidelity, "Resulting Stats");
            }
            None => {
                for label in ["Variant", "Archetype", "Design Team"] {
                    model.push_header_unavailable(&mut fidelity, label);
                }
                model.push_stat_unavailable(&mut fidelity, "Resulting Stats");
            }
        }

        let roots = build_root(ctx, &file, UPGRADE_ROOT);
        let area = overlay_area(&roots, ctx.screen);
        finish(UPGRADE_VIEW_ID, self.policy(), roots, model, area, fidelity)
    }
    crate::impl_view_any!();
}

/// The first equipment variant of the observed country, any family (deterministic: family
/// order then sorted key). `None` → the upgrade window badges all binds unavailable.
fn first_variant_any_family<'a>(snapshot: &'a Snapshot, tag: &str) -> Option<(String, &'a Value)> {
    let obj = equipment_variants(snapshot)?;
    for fam_tok in ["land_vehicle", "plane", "ship", "land_other"] {
        let mut keys: Vec<&String> = obj.keys().collect();
        keys.sort();
        for k in keys {
            let vv = &obj[k];
            let creator = vv.get("creator").and_then(Value::as_str);
            let family = vv.get("family").and_then(Value::as_str);
            if creator == Some(tag) && family == Some(fam_tok) {
                return Some((k.clone(), vv));
            }
        }
    }
    None
}

/// The per-country experience channel for a variant family token.
fn family_xp_field(fam: &str) -> &'static str {
    match fam {
        "plane" => "military.xp.air",
        "ship" => "military.xp.navy",
        _ => "military.xp.army",
    }
}

fn open_upgrade(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(EquipmentUpgradeDesignerView::new())
}

register_view!(ViewDescriptor {
    id: UPGRADE_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(UPGRADE_FILE, UPGRADE_ROOT)),
    required_domains: &["countries", "designs"],
    route: Some(MenuRoute {
        cluster: MenuCluster::Designers,
        label: "Equipment Upgrade",
        order: 5
    }),
    shortcut: None,
    open: open_upgrade,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("designers.upgrade.stat_entry"),
    gui_template: GuiSymbolRef::template(UPGRADE_FILE, "equipment_designer_stat_entry"),
    required_domains: &["designs"],
});

/// Linkage anchor — the binary calls a per-cluster `link()` to force the `inventory`
/// submissions into the final binary (symmetric with the other clusters).
pub fn link() {}

#[cfg(test)]
mod tests;
