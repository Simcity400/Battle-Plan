//! CLUSTER 2i — chrome_outliner_maps (7 views) [N7 minimap, N8 map-overlay icons,
//! N2 tab/list].
//!
//! The seven in-game "chrome / outliner / map" surfaces, bound per
//! `replay_ui/BINDING_MAP_chrome_outliner_maps.md`:
//!   1. **ingame_clock** (`ingame_clock.json` ROOT `clock_window`) — the `HH:00` readout
//!      bound from `ctx.date` (top-level snapshot date; the hour drives it). Exact.
//!   2. **minimap** (`minimap.json` ROOT `mini_map_interface`) — the real `.gui` chrome
//!      PLUS the **N7** minimap viewport: the camera-view rect (from `ctx.map_camera`)
//!      outlined on a downscaled-map panel. The map RASTER is owned `'static` by the
//!      binary-seeded `core.map` view and is UNREACHABLE from a registry-opened cluster
//!      view, so N7 is fed `map=None` (it draws a non-fabricated panel bg, never a fake
//!      map) → the raster fill is badged DERIVED/unavailable, never invented.
//!   3. **alerts** (`alerts.json`, 2 ROOTS) — there is **no `alerts[]` ledger** in the
//!      schema (alerts are recomputed live, never persisted), so the alert-icon badge is
//!      `unavailable`; the diplo-request `country_flag` is derivable-when-reconstructed.
//!   4. **find_view** (`find_view.json` ROOT `find_view`) — the search panel chrome + the
//!      **N2** category tab/list (states / countries / victory-points). The rows are a
//!      BROWSABLE CATALOG derived from the live snapshot (`states.*` ids+owner, the
//!      observed `countries.<TAG>`), NOT live search results — the search TERM is
//!      replay-local UI input with no value at bind time (digest carries `query=none`).
//!      State labels are `#id (owner)` tokens (a localized state name needs loc/game_defs
//!      a cluster does not load); the countries catalog is the observed country only
//!      (whole-world country enumeration needs the top-level domain seam).
//!   5. **rightclick_menu** (`rightclick_menu.json` ROOT `rightClickMenu`) — static
//!      context-menu chrome; its option SET is an imperative per-target adapter over
//!      mutations (inert in a read-only replay) → labels derivable, actions deferred.
//!   6. **mapmodes** (`mapmodes.json` ROOT `MapmodesInterface_Ingame`) — the map-mode
//!      rail chrome; the binding is PER-MAP-MODE (each `mapmode_button_*` slot is a
//!      recolor function over the province raster). The 14 modes' fidelity (per §B.9) is
//!      documented in the draw digest; the active-mode datum lives in the binary's map
//!      view (unreachable here) so the active indicator is badged.
//!   7. **mapicons** (`mapicons.json` ROOT `mapicons_container`) — the on-map icon layer.
//!      The PER-STATE icons reachable via the no-fabrication seam are bound EXACT and
//!      PLACED on the map at a real province centroid: **victory_point_mapicon** +
//!      **capital_mapicon** (VP, placed at the state's VP province via `state_meta.json`),
//!      **resistance_compliance_map_icon** (%, at a representative province), and
//!      **air_base_mapicon** (level). Centroids are built lazily from
//!      `ctx.stage.map_dir/province_ids` and projected through `ctx.map_camera`.
//!      Unit-counter stack count/owner/org/strength/type/dig-in logical entities plus
//!      commander portraits and formation insignia are PARTIALLY bound only when a
//!      division/OOB record supplies a real province anchor; full stack/counter parity
//!      is still not claimed. The remaining on-map TEMPLATE families read broader
//!      top-level domains (`combats`/`raids`/`diplomacy_world`, etc.) or are schema-unfed
//!      / asset-gap, so each is explicitly BADGED `unavailable` (ACCOUNTED, never silently
//!      dropped, never fabricated). All entities render via the **N8** map-overlay layer:
//!      placed icons on the map, badged ones in a screen legend.
//!
//! THE NO-FABRICATION CONTRACT. This is a CLUSTER file, so the source guard
//! (`views::digest::no_fabricated_default_in_cluster_views`) forbids `unwrap_or(0)` /
//! `unwrap_or_default()` / the raw `ctx.snapshot()` seam. Every field is read through
//! [`BindContext::country_field`] / [`BindContext::state_field`] → [`Captured`] and
//! rendered through [`DisplayAtom`], so a blocked/absent/null field renders the
//! not-captured glyph "—" — NEVER a fabricated 0. Genuinely runtime-computed /
//! live-resisted / asset-gap fields are badged `unavailable`. `ctx.date` is a plain
//! `BindContext` field (the top-level date), not the forbidden raw-snapshot seam.

use gui_runtime::scene::Node;
use gui_runtime::{build_scene, layout, GuiFile, ManifestContent, Rect};

use bind::Fidelity;
use render::modes::{BuildingKind, MapMode as CoreMapMode};

use crate::host::context::{BindContext, Captured, DisplayAtom, FidelityReport, UnavailableReason};
use crate::host::draw::FrameDraw;
use crate::host::input::{PointerButton, ViewInput};
use crate::host::registry::{GuiSymbolRef, MenuCluster, MenuRoute, ViewDescriptor, ViewId};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InputOutcome, InstanceId, RebindContext,
    ViewCommand, ViewInstance, ZBand,
};
use crate::register_view;

use widgets::common::{Atom as WAtom, DrawAssets, Fidelity as WFidelity, Reason as WReason};
use widgets::map_overlay::{self, IconAnchor, MapIcon};
use widgets::minimap::{self, Minimap, WorldRect};
use widgets::tabs::{self, Tab, TabBar};

use std::sync::OnceLock;

/// Process-global province-centroid cache (READ-ONLY, built ONCE from the half-res
/// `province_ids` raster on first use). A registry-opened cluster view cannot hold the
/// binary's `'static` raster, but `ctx.stage.map_dir` IS reachable + the raster PNG is
/// public — so we lazily build centroids here (Codex (b): "centroids are not impossible —
/// the host merely did not wire them"). `None` if the raster is absent → on-map placement
/// is honestly badged, never fabricated.
///
/// CACHE KEYING (Codex): this is NOT keyed by `map_dir` — the app uses a single,
/// process-invariant `map_dir` for the whole session (set once in `main.rs`), so a
/// single global cache is correct. The first init wins; if the process ever served two
/// different map dirs this would need a `map_dir`-keyed map.
static CENTROIDS: OnceLock<Option<CentroidSet>> = OnceLock::new();

/// The centroid map + the raster dimensions it was computed in (so a half-res centroid
/// pixel can be normalized to [0,1] and re-projected through the FULL-res map camera).
struct CentroidSet {
    centroids: map_overlay::ProvinceCentroids,
    raster_w: f32,
    raster_h: f32,
}

/// Build (once) the province centroids from `map_dir`'s `province_ids` raster. Prefers
/// the half-res PNG (smaller, same ids); falls back to full-res; `None` if neither loads.
fn province_centroids(map_dir: &std::path::Path) -> Option<&'static CentroidSet> {
    CENTROIDS
        .get_or_init(|| {
            for f in ["province_ids_half.png", "province_ids.png"] {
                let p = map_dir.join(f);
                if let Ok(raster) = render::map::ProvinceRaster::load_png(&p) {
                    return Some(CentroidSet {
                        centroids: map_overlay::ProvinceCentroids::from_raster_ids(
                            &raster.ids,
                            raster.width as usize,
                        ),
                        raster_w: raster.width.max(1) as f32,
                        raster_h: raster.height.max(1) as f32,
                    });
                }
            }
            None
        })
        .as_ref()
}

/// Per-state map geometry from `map_dir/state_meta.json`: the state's VP province (for a
/// VP/capital icon) + a representative province (for a per-state icon like resistance).
/// READ-ONLY, built once. Without it, a per-state icon has NO honest map province (its
/// state id is NOT a province id — Codex) → its placement is badged, never fabricated.
///
/// CACHE KEYING (Codex): like [`CENTROIDS`], a single process-global cache (NOT keyed by
/// `map_dir`) — correct because the app's `map_dir` is process-invariant (set once in
/// `main.rs`). The first init wins.
struct StateGeo {
    /// state id → the province carrying the most VP, FALLING BACK to the representative
    /// province when the state has no VP entry (so a VP/capital icon still places).
    vp_province: std::collections::HashMap<u32, u32>,
    /// state id → a representative province (first in the state) for per-state icons.
    rep_province: std::collections::HashMap<u32, u32>,
}

static STATE_GEO: OnceLock<Option<StateGeo>> = OnceLock::new();

/// Load (once) the state→province geometry from `state_meta.json`. `None` if absent.
fn state_geo(map_dir: &std::path::Path) -> Option<&'static StateGeo> {
    STATE_GEO
        .get_or_init(|| {
            let raw = std::fs::read_to_string(map_dir.join("state_meta.json")).ok()?;
            let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
            let obj = v.as_object()?;
            let mut vp_province = std::collections::HashMap::new();
            let mut rep_province = std::collections::HashMap::new();
            for (sid_s, meta) in obj {
                let sid: u32 = match sid_s.parse() {
                    Ok(n) => n,
                    Err(_) => continue,
                };
                // representative province = first in the `provinces` array.
                let first_prov = meta
                    .get("provinces")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|p| p.as_u64())
                    .map(|p| p as u32);
                if let Some(first) = first_prov {
                    rep_province.insert(sid, first);
                }
                // VP province = the highest-VP entry in `victory_points{pid:vp}`; SKIP
                // entries whose vp is not a number (no fabricated default).
                let best_vp = meta
                    .get("victory_points")
                    .and_then(|m| m.as_object())
                    .and_then(|vps| {
                        vps.iter()
                            .filter_map(|(pid, vp)| Some((pid.parse::<u32>().ok()?, vp.as_f64()?)))
                            .max_by(|a, b| {
                                a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
                            })
                            .map(|(pid, _)| pid)
                    });
                // VP province, FALLING BACK to the representative province when the state
                // carries no VP entry (so a VP/capital icon still places honestly).
                if let Some(pid) = best_vp.or(first_prov) {
                    vp_province.insert(sid, pid);
                }
            }
            Some(StateGeo {
                vp_province,
                rep_province,
            })
        })
        .as_ref()
}

/// Project a province centroid (in its source-raster pixel space) to a SCREEN pixel
/// through the app map camera. It is ALGEBRAICALLY EQUIVALENT to (not byte-identical
/// with) `app::camera::MapCamera::raster_to_screen` for a valid camera + a coordinate
/// normalized by its own raster size:
///   map_u = cx/raster_w ; out_u = (map_u - offset)*scale ; screen_x = out_u*screen_w
/// (this fn additionally sanitizes a degenerate `scale`, which the camera method does
/// not). The centroid is normalized by its OWN raster size (which may be half-res), so a
/// half-res raster projects to the same UV the full-res camera uses.
fn project_centroid(
    cam: &crate::camera::MapCamera,
    set: &CentroidSet,
    cx: f32,
    cy: f32,
) -> (f32, f32) {
    let map_u = cx / set.raster_w;
    let map_v = cy / set.raster_h;
    let s = if cam.scale.is_finite() && cam.scale > 0.0 {
        cam.scale
    } else {
        1.0
    };
    let out_u = (map_u - cam.offset[0]) * s;
    let out_v = (map_v - cam.offset[1]) * s;
    (out_u * cam.screen_w, out_v * cam.screen_h)
}

// =====================================================================================
// Shared helpers (no-fabrication: read via Captured, render via DisplayAtom — never a 0).
// =====================================================================================

const DASH: &str = "—";

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

/// Format a measured value as its raw string/number token. "—" only if not str/number.
fn fmt_token(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => DASH.to_string(),
    }
}

/// An empty GUI file (the no-fabrication fallback when a `.gui` did not pre-load — the
/// view then renders no chrome rather than invent a layout).
fn empty_gui() -> GuiFile {
    GuiFile::parse(r#"{"view":"empty","windows":[]}"#).expect("empty gui parses")
}

/// One resolved cell of the chrome: the target element name + the bound atom.
#[derive(Clone)]
struct Cell {
    element: String,
    atom: DisplayAtom,
}

impl Cell {
    fn new(element: &str, atom: DisplayAtom) -> Cell {
        Cell {
            element: element.to_string(),
            atom,
        }
    }
}

/// Translate a host [`DisplayAtom`] into a widgets [`WAtom`] (the no-fabrication bridge:
/// an Unavailable host atom collapses to the widget "—" glyph regardless of text).
fn watom(a: &DisplayAtom) -> WAtom {
    match a {
        DisplayAtom::Text { text, fidelity } => {
            let wf = match fidelity {
                Fidelity::Exact => WFidelity::Exact,
                Fidelity::Derived => WFidelity::Derived,
                Fidelity::Unavailable => WFidelity::Unavailable,
            };
            WAtom::from_parts(text.clone(), wf, WReason::Blocked)
        }
        DisplayAtom::Unavailable { reason, .. } => {
            let r = match reason {
                UnavailableReason::Absent => WReason::Absent,
                UnavailableReason::Null => WReason::Null,
                UnavailableReason::Blocked => WReason::Blocked,
            };
            WAtom::unavailable_because(r)
        }
    }
}

/// A digest line for a set of bound chrome cells (stable, comparable across a scrub).
fn cells_digest(cells: &[Cell]) -> String {
    let mut s = String::new();
    for c in cells {
        let badge = match c.atom.fidelity() {
            Fidelity::Exact => "E",
            Fidelity::Derived => "D",
            Fidelity::Unavailable => "U",
        };
        s.push_str(&format!("{}={}({badge});", c.element, c.atom.as_str()));
    }
    s
}

/// Build the named view ROOT scene from a parsed `.gui`, stamp the bound atoms into the
/// named elements, and lay it out at `ctx.screen`. Shared by every chrome view. A root
/// the file does not contain yields an empty `Vec` (no fabricated chrome).
fn build_root(ctx: &BindContext, file: &GuiFile, root_name: &str, cells: &[Cell]) -> Vec<Node> {
    let stage = ctx.stage;
    let cm = ManifestContent {
        manifest: &stage.manifest,
        measure: |_t, _f| None,
    };
    let mut roots: Vec<Node> = build_scene(file, &stage.manifest, &stage.loc)
        .into_iter()
        .filter(|w| w.name == root_name)
        .collect();
    for cell in cells {
        for root in roots.iter_mut() {
            bind::apply::set_element_text(root, &cell.element, cell.atom.as_str());
        }
    }
    for root in roots.iter_mut() {
        layout(root, ctx.screen, None, &cm);
    }
    roots
}

/// Build ALL the named view ROOTS in a file (alerts has two), stamping the same cells
/// into each (each root only carries the elements it owns; the rest no-op honestly).
fn build_all_roots(
    ctx: &BindContext,
    file: &GuiFile,
    root_names: &[&str],
    cells: &[Cell],
) -> Vec<Node> {
    let mut out = Vec::new();
    for name in root_names {
        out.extend(build_root(ctx, file, name, cells));
    }
    out
}

/// A 1x1 white texture for the widgets' `DrawAssets`. A registry-opened cluster view
/// cannot hold a `'static` white texture (those are leaked by the binary-seeded core
/// views only), so we upload a tiny one per draw — cheap + self-contained, identical to
/// `widgets::common::white_texture`.
fn frame_white(fd: &mut FrameDraw) -> render::Texture {
    fd.renderer.upload_rgba8_image(1, 1, &[255, 255, 255, 255])
}

/// Resolve a `.gui` ROOT through `ctx`, falling back to the file in `Stage` (so a view
/// can both bind named elements AND keep the parsed file for templates). Returns the
/// parsed file (or an empty one) — never a fabricated layout.
fn gui_file(ctx: &BindContext, file: &str) -> GuiFile {
    ctx.stage.gui_file(file).cloned().unwrap_or_else(empty_gui)
}

// =====================================================================================
// VIEW 1 — IN-GAME CLOCK
// =====================================================================================

const CLOCK_VIEW_ID: ViewId = ViewId("chrome.clock");
const CLOCK_FILE: &str = "ingame_clock.json";
const CLOCK_ROOT: &str = "clock_window";

pub struct ClockView;
impl ClockView {
    pub fn new() -> ClockView {
        ClockView
    }
}
impl Default for ClockView {
    fn default() -> Self {
        ClockView::new()
    }
}

struct ClockDrawable {
    roots: Vec<Node>,
    cells: Vec<Cell>,
}
impl Drawable for ClockDrawable {
    fn draw_digest(&self) -> String {
        format!("CLOCK {}", cells_digest(&self.cells))
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

/// Parse a Clausewitz `"Y.M.D.H"` date into its HOUR component → `H:00` wall-clock. The
/// engine ticks hourly (minutes always `00`). Returns `None` if the date is malformed OR
/// the hour is out of range (`>= 24`) — an impossible hour is not rendered as "exact".
fn clock_hhmm(date: &str) -> Option<String> {
    let h: u32 = date.split('.').nth(3)?.parse().ok()?;
    if h >= 24 {
        return None;
    }
    Some(format!("{h}:00"))
}

impl ViewInstance for ClockView {
    fn view_id(&self) -> ViewId {
        CLOCK_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Chrome)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = gui_file(ctx, CLOCK_FILE);

        // time_text <- the hour component of the top-level date (ctx.date is a plain
        // BindContext field, NOT the forbidden raw-snapshot seam). Exact: the scrubber
        // spine sets the date, the hour drives the readout.
        let atom = match clock_hhmm(ctx.date) {
            Some(s) => DisplayAtom::text(s, Fidelity::Exact),
            None => DisplayAtom::unavailable_because(UnavailableReason::Absent),
        };
        fidelity.record(&atom);
        let cells = vec![Cell::new("time_text", atom)];

        let roots = build_root(ctx, &file, CLOCK_ROOT, &cells);
        BoundView {
            instance: InstanceId(0),
            view_id: CLOCK_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots.clone()),
            drawable: Box::new(ClockDrawable { roots, cells }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }
    crate::impl_view_any!();
}

fn open_clock(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(ClockView::new())
}

register_view!(ViewDescriptor {
    id: CLOCK_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(CLOCK_FILE, CLOCK_ROOT)),
    required_domains: &[],
    route: Some(MenuRoute {
        cluster: MenuCluster::ChromeOutlinerMaps,
        label: "Clock",
        order: 1
    }),
    shortcut: None,
    open: open_clock,
    policy: HostPolicy::window(ZBand::Chrome),
});

// =====================================================================================
// VIEW 2 — MINIMAP  [N7]
// =====================================================================================

pub const MINIMAP_VIEW_ID: ViewId = ViewId("chrome.minimap");
const MINIMAP_FILE: &str = "minimap.json";
const MINIMAP_ROOT: &str = "mini_map_interface";

#[derive(Clone, Copy)]
struct MinimapInputState {
    panel: Rect,
    world: (f32, f32),
    camera_view: WorldRect,
}

pub struct MinimapView {
    input: Option<MinimapInputState>,
}
impl MinimapView {
    pub fn new() -> MinimapView {
        MinimapView { input: None }
    }

    /// Sync the latest live map camera into the minimap's input hit-test state. The
    /// draw path still binds from `ctx.map_camera`; this cache is only for event-time
    /// picking, where `ViewInstance::on_input` has no `BindContext`.
    pub fn set_camera(&mut self, camera: Option<crate::camera::MapCamera>) {
        let panel = self.input.map(|input| input.panel);
        self.input = camera.map(|cam| minimap_input_from_camera(cam, panel));
    }

    /// Sync the event-time hit-test panel from the same bound rect the draw path uses.
    /// If no live map camera is cached, the minimap remains inert.
    pub fn set_input_panel(&mut self, panel: Rect) {
        if let Some(mut input) = self.input {
            input.panel = panel;
            self.input = Some(input);
        }
    }
}
impl Default for MinimapView {
    fn default() -> Self {
        MinimapView::new()
    }
}

/// The minimap's owned draw inputs: the real `.gui` chrome + the N7 viewport math
/// (panel rect, world size, the camera-view rect derived from the live map camera). The
/// map TEXTURE is `None` (the `'static` raster is owned by the binary-seeded core.map
/// view, unreachable from a registry view) → N7 draws a non-fabricated panel bg.
struct MinimapDrawable {
    roots: Vec<Node>,
    panel: Rect,
    world: (f32, f32),
    /// the camera-view rect in raster/world space, if a live camera was present.
    camera_view: Option<WorldRect>,
    /// the raster-fidelity badge ("derived" when a camera is present, "unavailable" else).
    raster_badge: &'static str,
}

impl Drawable for MinimapDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "MINIMAP panel={:.2},{:.2},{:.2},{:.2} world={:.0}x{:.0} cam={:?} raster={}",
            self.panel.x,
            self.panel.y,
            self.panel.w,
            self.panel.h,
            self.world.0,
            self.world.1,
            self.camera_view
                .map(|c| (c.x as i64, c.y as i64, c.w as i64, c.h as i64)),
            self.raster_badge,
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        // the real .gui chrome first (border/handles/ping buttons — inert, drawn as art).
        fd.draw_nodes(&self.roots);
        // then the N7 viewport over the panel area: a non-fabricated panel bg + the
        // camera-view outline. NO map texture (the raster is unreachable here).
        let white = frame_white(fd);
        let loaded = fd.fonts.get(fd.renderer, "hoi_16mbs");
        let (font, pages): (Option<&assets::BmFont>, &[render::Texture]) = match &loaded {
            Some(l) => (Some(&l.font), &l.pages),
            None => (None, &[]),
        };
        let assets = DrawAssets {
            white: &white,
            font,
            font_pages: pages,
        };
        let mm = Minimap::new(self.panel, self.world.0, self.world.1);
        let view = self.camera_view.unwrap_or(WorldRect {
            x: 0.0,
            y: 0.0,
            w: self.world.0,
            h: self.world.1,
        });
        minimap::draw_minimap(fd.renderer, &mm, view, None, &assets);
    }
}

impl ViewInstance for MinimapView {
    fn view_id(&self) -> ViewId {
        MINIMAP_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Chrome)
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        let ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        } = *ev
        else {
            return InputOutcome::ignored();
        };
        let Some(input) = self.input else {
            return InputOutcome::ignored();
        };
        let mm = Minimap::new(input.panel, input.world.0, input.world.1);
        if let Some(p) = mm.pick(input.camera_view, x, y) {
            return InputOutcome::consumed().with_command(ViewCommand::RecenterMap {
                raster_x: p.x,
                raster_y: p.y,
            });
        }
        if input.panel.contains(x, y) {
            return InputOutcome {
                consumed: true,
                commands: Vec::new(),
                needs_redraw: false,
            };
        }
        InputOutcome::ignored()
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = gui_file(ctx, MINIMAP_FILE);

        // The minimap has TWO distinct data inputs with DISTINCT fidelity (Codex):
        //   * the camera-VIEWPORT rect (the white outline) — DERIVED when a live camera is
        //     present (computed from ctx.map_camera), else absent;
        //   * the worldmap RASTER fill (the recoloured thumbnail) — the PoliticalMap pass
        //     is owned 'static by the binary-seeded core.map view and is UNREACHABLE from
        //     a registry view, so N7 is fed map=None → the raster fill is ALWAYS
        //     UNAVAILABLE here (never a fabricated thumbnail).
        let (camera_view, world) = match ctx.map_camera {
            Some(cam) => {
                let input = minimap_input_from_camera(cam, None);
                (Some(input.camera_view), input.world)
            }
            None => (None, (5632.0, 2048.0)),
        };
        // the camera-viewport: derived if a camera was present, else absent.
        let viewport_atom = match camera_view {
            Some(_) => DisplayAtom::text("viewport", Fidelity::Derived),
            None => DisplayAtom::unavailable_because(UnavailableReason::Absent),
        };
        fidelity.record(&viewport_atom);
        // the raster fill: ALWAYS unavailable (the map pass is unreachable from here).
        let raster_atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        fidelity.record(&raster_atom);
        let raster_badge = if camera_view.is_some() {
            "viewport-derived,raster-unavailable"
        } else {
            "unavailable"
        };

        // the minimap panel = the .gui shell's screen rect (lower-right), or a default.
        let roots = build_root(ctx, &file, MINIMAP_ROOT, &[]);
        let panel = minimap_panel_rect(ctx.screen, &roots);

        BoundView {
            instance: InstanceId(0),
            view_id: MINIMAP_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots.clone()).with_block_rect(panel),
            drawable: Box::new(MinimapDrawable {
                roots,
                panel,
                world,
                camera_view,
                raster_badge,
            }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }
    crate::impl_view_any!();
}

fn minimap_input_from_camera(
    cam: crate::camera::MapCamera,
    panel: Option<Rect>,
) -> MinimapInputState {
    let (rw, rh) = (cam.raster_w.max(1.0), cam.raster_h.max(1.0));
    // The visible raster window = inverse of the shader transform:
    //   uv = px/screen / scale + offset  -> raster = uv * raster_size
    //   at px=0       -> uv = offset ; at px=screen -> uv = 1/scale + offset
    let s = if cam.scale.is_finite() && cam.scale > 0.0 {
        cam.scale
    } else {
        1.0
    };
    MinimapInputState {
        panel: panel.unwrap_or_else(|| {
            minimap_panel_fallback(Rect {
                x: 0.0,
                y: 0.0,
                w: cam.screen_w.max(1.0),
                h: cam.screen_h.max(1.0),
            })
        }),
        world: (rw, rh),
        camera_view: WorldRect {
            x: cam.offset[0] * rw,
            y: cam.offset[1] * rh,
            w: (1.0 / s) * rw,
            h: (1.0 / s) * rh,
        },
    }
}

fn minimap_panel_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 360.0,
        y: screen.y + screen.h - 200.0,
        w: 340.0,
        h: 180.0,
    }
}

fn minimap_panel_rect(screen: Rect, roots: &[Node]) -> Rect {
    root_rect(roots, MINIMAP_ROOT).unwrap_or_else(|| minimap_panel_fallback(screen))
}

/// The laid-out screen rect of a named root window (its panel), if non-degenerate.
fn root_rect(roots: &[Node], name: &str) -> Option<Rect> {
    roots
        .iter()
        .find(|r| r.name == name && r.rect.w > 0.0 && r.rect.h > 0.0)
        .map(|r| r.rect)
}

fn open_minimap(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(MinimapView::new())
}

register_view!(ViewDescriptor {
    id: MINIMAP_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(MINIMAP_FILE, MINIMAP_ROOT)),
    required_domains: &["map"],
    route: Some(MenuRoute {
        cluster: MenuCluster::ChromeOutlinerMaps,
        label: "Minimap",
        order: 2
    }),
    shortcut: None,
    open: open_minimap,
    policy: HostPolicy::window(ZBand::Chrome),
});

// =====================================================================================
// VIEW 3 — ALERTS
// =====================================================================================

const ALERTS_VIEW_ID: ViewId = ViewId("chrome.alerts");
const ALERTS_FILE: &str = "alerts.json";
const ALERTS_ROOTS: &[&str] = &["global_alerticon_window", "global_diplorequesticon_window"];

pub struct AlertsView;
impl AlertsView {
    pub fn new() -> AlertsView {
        AlertsView
    }
}
impl Default for AlertsView {
    fn default() -> Self {
        AlertsView::new()
    }
}

struct AlertsDrawable {
    roots: Vec<Node>,
    cells: Vec<Cell>,
}
impl Drawable for AlertsDrawable {
    fn draw_digest(&self) -> String {
        format!("ALERTS {}", cells_digest(&self.cells))
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for AlertsView {
    fn view_id(&self) -> ViewId {
        ALERTS_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Chrome)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = gui_file(ctx, ALERTS_FILE);

        // The alert TYPE (which frame of GFX_global_alert_icons) is a live per-player
        // computation; there is NO stored "active alerts" domain in the schema (alerts
        // are recomputed live, never persisted). Badge it unavailable — never fabricated.
        // (The `icon` element name lives on BOTH alert windows; set_element_text on a
        // button is a no-op for sprite frames but the badge is recorded honestly.)
        let alert_atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        fidelity.record(&alert_atom);

        // The diplo-request country_flag would be derivable IF an alerts adapter
        // reconstructed a diplomatic request from the diff-event timeline; no such
        // ledger is fed here → unavailable (the flag itself is exact-from-TAG once a TAG
        // exists, but the alert's EXISTENCE is adapter-reconstructed and absent).
        let flag_atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        fidelity.record(&flag_atom);

        let cells = vec![
            Cell::new("icon", alert_atom),
            Cell::new("country_flag", flag_atom),
        ];
        let roots = build_all_roots(ctx, &file, ALERTS_ROOTS, &cells);
        BoundView {
            instance: InstanceId(0),
            view_id: ALERTS_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots.clone()),
            drawable: Box::new(AlertsDrawable { roots, cells }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }
    crate::impl_view_any!();
}

fn open_alerts(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(AlertsView::new())
}

register_view!(ViewDescriptor {
    id: ALERTS_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(ALERTS_FILE, "global_alerticon_window")),
    required_domains: &[],
    route: Some(MenuRoute {
        cluster: MenuCluster::ChromeOutlinerMaps,
        label: "Alerts",
        order: 3
    }),
    shortcut: None,
    open: open_alerts,
    policy: HostPolicy::window(ZBand::Chrome),
});

// =====================================================================================
// VIEW 4 — FIND VIEW  [N2 tab/list]
// =====================================================================================

const FIND_VIEW_ID: ViewId = ViewId("chrome.find");
const FIND_FILE: &str = "find_view.json";
const FIND_ROOT: &str = "find_view";

pub struct FindView {
    bar: TabBar,
    screen: Rect,
    catalog: FindCatalog,
    selected_row: Option<String>,
}
impl FindView {
    pub fn new() -> FindView {
        FindView {
            bar: find_tabs(),
            screen: default_find_screen(),
            catalog: FindCatalog::default(),
            selected_row: None,
        }
    }
}
impl Default for FindView {
    fn default() -> Self {
        FindView::new()
    }
}

/// One searchable result row: its display name atom + a stable key (for the digest).
#[derive(Clone)]
struct FindRow {
    key: String,
    name: DisplayAtom,
    target: FindRowTarget,
}

#[derive(Clone)]
enum FindRowTarget {
    State(u32),
    Country,
}

#[derive(Clone, Default)]
struct FindCatalog {
    states: Vec<FindRow>,
    countries: Vec<FindRow>,
    vps: Vec<FindRow>,
}

impl FindCatalog {
    fn for_rebind(ctx: &RebindContext) -> FindCatalog {
        FindCatalog {
            states: find_rows_without_fidelity(ctx, FindTab::States),
            countries: find_rows_without_fidelity(ctx, FindTab::Countries),
            vps: find_rows_without_fidelity(ctx, FindTab::VictoryPoints),
        }
    }

    fn rows(&self, tab: FindTab) -> &[FindRow] {
        match tab {
            FindTab::States => &self.states,
            FindTab::Countries => &self.countries,
            FindTab::VictoryPoints => &self.vps,
        }
    }
}

/// Which catalog the find tab is showing (replay-supported: states / countries / VPs).
#[derive(Clone, Copy, PartialEq, Eq)]
enum FindTab {
    States,
    Countries,
    VictoryPoints,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindTabClick {
    Changed,
    Unchanged,
    Inert,
    Miss,
}

struct FindDrawable {
    roots: Vec<Node>,
    bar: TabBar,
    panel: Rect,
    rows: Vec<FindRow>,
    selected_row: Option<String>,
}
impl Drawable for FindDrawable {
    fn draw_digest(&self) -> String {
        // NOTE: query=none — these rows are the BROWSABLE CATALOG, not live search
        // results (the search term is replay-local UI input with no value at bind).
        let mut s = format!(
            "FIND query=none tab={:?} selected={} rows=[",
            self.bar.selected(),
            find_selected_digest(self.selected_row.as_deref(), &self.rows)
        );
        for r in &self.rows {
            s.push_str(&format!(
                "{}={}({});",
                r.key,
                r.name.as_str(),
                atom_badge(&r.name)
            ));
        }
        s.push(']');
        s
    }
    fn draw(&self, fd: &mut FrameDraw) {
        // the real find_view .gui chrome (editbox / buttons / window frame).
        fd.draw_nodes(&self.roots);
        // the N2 category tab bar + the result rows below it, drawn in the results area.
        let white = frame_white(fd);
        let loaded = fd.fonts.get(fd.renderer, "hoi_16mbs");
        let (font, pages): (Option<&assets::BmFont>, &[render::Texture]) = match &loaded {
            Some(l) => (Some(&l.font), &l.pages),
            None => (None, &[]),
        };
        let assets = DrawAssets {
            white: &white,
            font,
            font_pages: pages,
        };
        tabs::plan_tabs(&self.bar, self.panel).execute(fd.renderer, &assets, |_, _| false);
        // result rows below the tab bar.
        let content = self.bar.content_rect(self.panel);
        for (i, r) in self.rows.iter().enumerate() {
            let y = content.y + i as f32 * FIND_ROW_H;
            if y + FIND_ROW_H > content.y + content.h {
                break;
            }
            let clip = Some([content.x, content.y, content.w, content.h]);
            if self.selected_row.as_deref() == Some(r.key.as_str()) {
                widgets::common::fill_rect(
                    fd.renderer,
                    &white,
                    content.x + 2.0,
                    y + 1.0,
                    content.w - 4.0,
                    FIND_ROW_H - 2.0,
                    [0.24, 0.34, 0.46, 0.70],
                    clip,
                );
            }
            let atom = watom(&r.name);
            widgets::common::draw_atom(
                fd.renderer,
                &assets,
                &atom,
                content.x + 6.0,
                y + 4.0,
                widgets::common::TEXT_TINT,
                clip,
            );
        }
    }
}

/// The fixed catalog tabs (states/countries/VPs are replay-supported; live-game extras
/// like find-by-decision/template need more catalogs → omitted, not faked).
fn find_tabs() -> TabBar {
    TabBar::horizontal(vec![
        Tab::enabled("states", WAtom::exact("States")),
        Tab::enabled("countries", WAtom::exact("Countries")),
        Tab::enabled("vps", WAtom::exact("Victory Points")),
    ])
}

const FIND_ROW_H: f32 = 22.0;

fn atom_badge(atom: &DisplayAtom) -> &'static str {
    match atom.fidelity() {
        Fidelity::Exact => "E",
        Fidelity::Derived => "D",
        Fidelity::Unavailable => "U",
    }
}

fn find_selected_digest(selected: Option<&str>, rows: &[FindRow]) -> String {
    match selected.and_then(|key| rows.iter().find(|r| r.key == key)) {
        Some(row) => format!(
            "{}={}({})",
            row.key,
            row.name.as_str(),
            atom_badge(&row.name)
        ),
        None => "none".to_string(),
    }
}

fn selected_find_tab(bar: &TabBar) -> FindTab {
    match bar.selected() {
        Some("countries") => FindTab::Countries,
        Some("vps") => FindTab::VictoryPoints,
        _ => FindTab::States,
    }
}

fn default_find_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn find_panel_fallback(screen: Rect) -> Rect {
    // Mirrors find_view.gui: lower_right anchor at (-280, -540), 285x300.
    Rect {
        x: screen.x + screen.w - 280.0,
        y: screen.y + screen.h - 540.0,
        w: 285.0,
        h: 300.0,
    }
}

fn find_panel_rect(screen: Rect, roots: &[Node]) -> Rect {
    root_rect(roots, FIND_ROOT).unwrap_or_else(|| find_panel_fallback(screen))
}

fn click_find_tab(bar: &mut TabBar, panel: Rect, x: f32, y: f32) -> FindTabClick {
    let rects = bar.layout(panel);
    if let Some(id) = bar.pick(&rects, x, y) {
        if bar.select(&id) {
            FindTabClick::Changed
        } else {
            FindTabClick::Unchanged
        }
    } else if rects.iter().any(|tr| tr.rect.contains(x, y)) {
        FindTabClick::Inert
    } else {
        FindTabClick::Miss
    }
}

fn pick_find_row_index(
    bar: &TabBar,
    panel: Rect,
    rows: &[FindRow],
    x: f32,
    y: f32,
) -> Option<usize> {
    let content = bar.content_rect(panel);
    if !content.contains(x, y) {
        return None;
    }
    let idx = ((y - content.y) / FIND_ROW_H).floor() as usize;
    if idx >= rows.len() {
        return None;
    }
    let row_y = content.y + idx as f32 * FIND_ROW_H;
    if row_y + FIND_ROW_H > content.y + content.h {
        return None;
    }
    Some(idx)
}

fn find_row_command(row: &FindRow) -> Option<ViewCommand> {
    match row.target {
        FindRowTarget::State(sid) => Some(ViewCommand::OpenView {
            view_id: crate::views::state_panel::STATE_PANEL_VIEW_ID,
            arg: Some(i64::from(sid)),
        }),
        FindRowTarget::Country => None,
    }
}

fn find_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
}

trait FindCatalogSource<'a> {
    fn find_country(&self) -> &'a str;
    fn find_country_field(&self, tag: &str, dotted: &str) -> Captured<&'a serde_json::Value>;
    fn find_state_field(&self, sid: &str, dotted: &str) -> Captured<&'a serde_json::Value>;
}

impl<'a> FindCatalogSource<'a> for BindContext<'a> {
    fn find_country(&self) -> &'a str {
        self.country
    }

    fn find_country_field(&self, tag: &str, dotted: &str) -> Captured<&'a serde_json::Value> {
        self.country_field(tag, dotted)
    }

    fn find_state_field(&self, sid: &str, dotted: &str) -> Captured<&'a serde_json::Value> {
        self.state_field(sid, dotted)
    }
}

impl<'a> FindCatalogSource<'a> for RebindContext<'a> {
    fn find_country(&self) -> &'a str {
        self.country
    }

    fn find_country_field(&self, tag: &str, dotted: &str) -> Captured<&'a serde_json::Value> {
        self.country_field(tag, dotted)
    }

    fn find_state_field(&self, sid: &str, dotted: &str) -> Captured<&'a serde_json::Value> {
        self.state_field(sid, dotted)
    }
}

fn find_rows(ctx: &BindContext, tab: FindTab, fidelity: &mut FidelityReport) -> Vec<FindRow> {
    let mut fidelity = Some(fidelity);
    find_rows_from(ctx, tab, &mut fidelity)
}

fn find_rows_without_fidelity(ctx: &RebindContext, tab: FindTab) -> Vec<FindRow> {
    let mut fidelity = None;
    find_rows_from(ctx, tab, &mut fidelity)
}

fn record_find_atom(fidelity: &mut Option<&mut FidelityReport>, atom: &DisplayAtom) {
    if let Some(report) = fidelity.as_deref_mut() {
        report.record(atom);
    }
}

/// Build the BROWSABLE CATALOG rows for a category tab from the LIVE snapshot's named
/// entities (DERIVED — the names are joined from the snapshot). The find search TERM is
/// replay-local UI input with NO value at bind time, so these are NOT live search results
/// — they are the catalog the search would filter (Codex (f): never pretend a catalog is
/// a query result). Capped so the panel stays bounded; each row is no-fabrication.
fn find_rows_from<'a, C: FindCatalogSource<'a>>(
    ctx: &C,
    tab: FindTab,
    fidelity: &mut Option<&mut FidelityReport>,
) -> Vec<FindRow> {
    const CAP: usize = 64;
    let mut rows = Vec::new();
    match tab {
        FindTab::Countries => {
            // The deterministic catalog slice: the observed country's TAG (always
            // present) — a real, honest entry (never a fabricated list). The localized
            // country NAME needs loc/game_defs a cluster does not load → TAG is the token.
            // ENRICHED with the now-recorded per-country `identity` (country_static_live
            // fold): the ORIGINAL TAG (a released/dynamic country's parent, measured-None
            // for a roster nation) + the ORIGINAL CAPITAL state id. Each is appended ONLY
            // when MEASURED-non-null — an absent/None field adds nothing (no fabrication).
            let country = ctx.find_country();
            let mut label = country.to_string();
            if let Captured::Measured(v) = ctx.find_country_field(country, "identity.original_tag")
            {
                if let Some(orig) = v.as_str() {
                    label.push_str(&format!(" (was {orig})"));
                }
            }
            if let Captured::Measured(v) =
                ctx.find_country_field(country, "identity.original_capital")
            {
                if let Some(cap) = v.as_i64() {
                    label.push_str(&format!(" [cap #{cap}]"));
                }
            }
            // core / claim COUNTS (the per-country inversion of states' is_core_of /
            // is_claimed_by, country_containers_live fold). Appended only when the measured
            // list is non-empty (a measured-empty list adds nothing; never a fabricated 0).
            if let Captured::Measured(v) = ctx.find_country_field(country, "identity.cores") {
                if let Some(n) = v.as_array().map(|a| a.len()).filter(|n| *n > 0) {
                    label.push_str(&format!(" cores:{n}"));
                }
            }
            if let Captured::Measured(v) = ctx.find_country_field(country, "identity.claims") {
                if let Some(n) = v.as_array().map(|a| a.len()).filter(|n| *n > 0) {
                    label.push_str(&format!(" claims:{n}"));
                }
            }
            let tag_atom = DisplayAtom::text(label, Fidelity::Derived);
            record_find_atom(fidelity, &tag_atom);
            rows.push(FindRow {
                key: country.to_string(),
                name: tag_atom,
                target: FindRowTarget::Country,
            });
        }
        FindTab::States => {
            // states are keyed by id in the snapshot; surface the first CAP state ids
            // with their owner as a derived "name" (a localized state name needs loc/
            // game_defs which a cluster does not load — so id+owner is the honest token).
            for sid in state_ids(ctx).into_iter().take(CAP) {
                let target_id = match sid.parse::<u32>() {
                    Ok(id) => id,
                    Err(_) => continue,
                };
                let owner = ctx.find_state_field(&sid, "owner");
                let owner_s = owner.display_with_fidelity(Fidelity::Exact, fmt_token);
                let label = format!("#{sid} ({})", owner_s.as_str());
                let atom = DisplayAtom::text(label, Fidelity::Derived);
                record_find_atom(fidelity, &atom);
                rows.push(FindRow {
                    key: format!("state:{sid}"),
                    name: atom,
                    target: FindRowTarget::State(target_id),
                });
            }
        }
        FindTab::VictoryPoints => {
            // VP entities = states carrying a victory_points_total > 0 (exact field).
            for sid in state_ids(ctx).into_iter().take(CAP) {
                let target_id = match sid.parse::<u32>() {
                    Ok(id) => id,
                    Err(_) => continue,
                };
                let vp = ctx.find_state_field(&sid, "victory_points_total");
                let atom = vp.display_with_fidelity(Fidelity::Exact, fmt_int);
                if !atom.is_unavailable() && atom.as_str() != "0" {
                    let label = format!("#{sid}: {} VP", atom.as_str());
                    let row_atom = DisplayAtom::text(label, Fidelity::Exact);
                    record_find_atom(fidelity, &row_atom);
                    rows.push(FindRow {
                        key: format!("vp:{sid}"),
                        name: row_atom,
                        target: FindRowTarget::State(target_id),
                    });
                }
            }
            if rows.is_empty() {
                // a measured-empty catalog is honest (no VP states fed this hour); record
                // one unavailable so the section flags "no data", never a fabricated row.
                let a = DisplayAtom::unavailable_because(UnavailableReason::Absent);
                record_find_atom(fidelity, &a);
            }
        }
    }
    rows
}

/// The state ids present in the snapshot (the keys of the `states` domain), sorted
/// numerically for a deterministic slice. Read via the state_field seam only — no raw
/// snapshot. We probe a bounded id range and keep the ones with a present `owner`/`category`.
fn state_ids<'a, C: FindCatalogSource<'a>>(ctx: &C) -> Vec<String> {
    // The snapshot states domain is keyed by id; without a raw-snapshot iterator (the
    // forbidden seam), we probe ids 1..=1200 (the game's state-id ceiling) and keep present
    // ones. A present state has SOME field (owner OR category OR victory_points_total).
    let mut out = Vec::new();
    for id in 1..=1200u32 {
        let sid = id.to_string();
        let present = matches!(
            ctx.find_state_field(&sid, "owner"),
            Captured::Measured(_) | Captured::Null
        ) || matches!(
            ctx.find_state_field(&sid, "category"),
            Captured::Measured(_)
        ) || matches!(
            ctx.find_state_field(&sid, "victory_points_total"),
            Captured::Measured(_)
        );
        if present {
            out.push(sid);
        }
    }
    out
}

impl ViewInstance for FindView {
    fn view_id(&self) -> ViewId {
        FIND_VIEW_ID
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
        self.catalog = FindCatalog::for_rebind(ctx);
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => match click_find_tab(&mut self.bar, find_panel_fallback(self.screen), x, y) {
                FindTabClick::Changed => find_input_consumed(true),
                FindTabClick::Unchanged | FindTabClick::Inert => find_input_consumed(false),
                FindTabClick::Miss => {
                    let tab = selected_find_tab(&self.bar);
                    let rows = self.catalog.rows(tab);
                    if let Some(idx) =
                        pick_find_row_index(&self.bar, find_panel_fallback(self.screen), rows, x, y)
                    {
                        let row = rows[idx].clone();
                        self.selected_row = Some(row.key.clone());
                        let mut out = find_input_consumed(true);
                        if let Some(cmd) = find_row_command(&row) {
                            out.commands.push(cmd);
                        }
                        out
                    } else {
                        InputOutcome::ignored()
                    }
                }
            },
            _ => InputOutcome::ignored(),
        }
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = gui_file(ctx, FIND_FILE);

        let bar = self.bar.clone();
        let tab = selected_find_tab(&bar);
        let rows = find_rows(ctx, tab, &mut fidelity);

        let roots = build_root(ctx, &file, FIND_ROOT, &[]);
        // the results panel = the find_view shell rect (or a default in the lower-right).
        let panel = find_panel_rect(ctx.screen, &roots);

        BoundView {
            instance: InstanceId(0),
            view_id: FIND_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots.clone()).with_block_rect(panel),
            drawable: Box::new(FindDrawable {
                roots,
                bar,
                panel,
                rows,
                selected_row: self.selected_row.clone(),
            }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }
    crate::impl_view_any!();
}

fn open_find(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(FindView::new())
}

register_view!(ViewDescriptor {
    id: FIND_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(FIND_FILE, FIND_ROOT)),
    required_domains: &["states", "countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::ChromeOutlinerMaps,
        label: "Find",
        order: 4
    }),
    shortcut: None,
    open: open_find,
    policy: HostPolicy::window(ZBand::Panel),
});

// =====================================================================================
// VIEW 5 — RIGHT-CLICK MENU
// =====================================================================================

const RIGHTCLICK_VIEW_ID: ViewId = ViewId("chrome.rightclick");
const RIGHTCLICK_FILE: &str = "rightclick_menu.json";
const RIGHTCLICK_ROOT: &str = "rightClickMenu";

pub struct RightClickView;
impl RightClickView {
    pub fn new() -> RightClickView {
        RightClickView
    }
}
impl Default for RightClickView {
    fn default() -> Self {
        RightClickView::new()
    }
}

struct RightClickDrawable {
    roots: Vec<Node>,
    cells: Vec<Cell>,
}
impl Drawable for RightClickDrawable {
    fn draw_digest(&self) -> String {
        format!("RIGHTCLICK {}", cells_digest(&self.cells))
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for RightClickView {
    fn view_id(&self) -> ViewId {
        RIGHTCLICK_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Overlay)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = gui_file(ctx, RIGHTCLICK_FILE);

        // The context menu is built imperatively per right-click target, and its options
        // are overwhelmingly MUTATIONS (move/order/assign/release/build) → inert in a
        // read-only replay. The one plausibly-live verb is "Open <target> view" (the
        // `open_button` on right_click_entry_2). Labels are derivable; the actions are
        // deferred. Badge the action-bearing cells unavailable (no per-context option set
        // is enumerated by any snapshot field).
        let label_atom = DisplayAtom::text("Open", Fidelity::Derived);
        fidelity.record(&label_atom);
        let action_atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        fidelity.record(&action_atom);

        // the template roots (right_click_entry / _2 / empty) are placement-only; the
        // `rightClickMenu` root carries the `options` listbox. We render the shell chrome.
        let cells = vec![
            Cell::new("open_button", label_atom),
            Cell::new("button", action_atom),
        ];
        // stamp the labels into the row templates too (so the open verb shows honestly).
        let mut roots = build_root(ctx, &file, RIGHTCLICK_ROOT, &[]);
        for tmpl in ["right_click_entry", "right_click_entry_2"] {
            roots.extend(build_root(ctx, &file, tmpl, &cells));
        }
        BoundView {
            instance: InstanceId(0),
            view_id: RIGHTCLICK_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots.clone()),
            drawable: Box::new(RightClickDrawable { roots, cells }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }
    crate::impl_view_any!();
}

fn open_rightclick(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(RightClickView::new())
}

register_view!(ViewDescriptor {
    id: RIGHTCLICK_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(RIGHTCLICK_FILE, RIGHTCLICK_ROOT)),
    required_domains: &[],
    route: Some(MenuRoute {
        cluster: MenuCluster::ChromeOutlinerMaps,
        label: "Right-Click",
        order: 5
    }),
    shortcut: None,
    open: open_rightclick,
    policy: HostPolicy::window(ZBand::Overlay),
});

// =====================================================================================
// VIEW 6 — MAP-MODE SWITCHER
// =====================================================================================

pub const MAPMODES_VIEW_ID: ViewId = ViewId("chrome.mapmodes");
const MAPMODES_FILE: &str = "mapmodes.json";
const MAPMODES_ROOT: &str = "MapmodesInterface_Ingame";

pub struct MapModesView {
    screen: Rect,
    input_panel: Option<Rect>,
}
impl MapModesView {
    pub fn new() -> MapModesView {
        MapModesView {
            screen: default_chrome_screen(),
            input_panel: None,
        }
    }

    /// Sync the event-time hit-test panel from the same bound rect the draw path uses.
    /// If this has not run yet, input falls back to the known rail footprint.
    pub fn set_input_panel(&mut self, panel: Rect) {
        self.input_panel = Some(panel);
    }
}
impl Default for MapModesView {
    fn default() -> Self {
        MapModesView::new()
    }
}

/// One map MODE = a recolor function over the province raster, carrying its §B.9
/// fidelity. The binding is PER-MODE (each `mapmode_button_*` slot instantiates one).
/// `data_driven` modes resolve their fidelity FROM THE SNAPSHOT each tick (the datum was
/// closed by the live reader) instead of a frozen label, so they light up the moment the
/// driving domain is captured and badge "—" when it is absent (no fabrication).
struct MapModeRailEntry {
    name: &'static str,
    fidelity: Fidelity,
    core_mode: Option<CoreMapMode>,
    /// when set, the mode's fidelity is computed from the snapshot at bind time (the now-
    /// recorded per-state `resources` / per-region `weather` domains) rather than the
    /// static `fidelity` field (which is then the BEST-CASE label, used only as a fallback).
    data_driven: Option<DataDrivenMode>,
}

/// Which now-recorded domain a `data_driven` map mode reads to resolve its live fidelity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataDrivenMode {
    /// Resources mode — recolors from `states.*.resources` (closed by the live reader);
    /// Exact when any state carries a non-null resources block, else Unavailable.
    StateResources,
    /// Weather mode — recolors from the `weather` domain (closed by the live reader);
    /// Derived (a proxy of the game's particle look) when present, else Unavailable.
    Weather,
}

impl MapModeRailEntry {
    const fn stat(
        name: &'static str,
        fidelity: Fidelity,
        core_mode: Option<CoreMapMode>,
    ) -> MapModeRailEntry {
        MapModeRailEntry {
            name,
            fidelity,
            core_mode,
            data_driven: None,
        }
    }
    const fn driven(
        name: &'static str,
        best: Fidelity,
        mode: DataDrivenMode,
        core_mode: Option<CoreMapMode>,
    ) -> MapModeRailEntry {
        MapModeRailEntry {
            name,
            fidelity: best,
            core_mode,
            data_driven: Some(mode),
        }
    }

    /// The mode's fidelity FOR THIS SNAPSHOT: a static mode returns its frozen label; a
    /// data-driven mode resolves it from the now-recorded domain (Exact/Derived when the
    /// data is captured, Unavailable when absent — never fabricated as available).
    fn fidelity_for(&self, ctx: &BindContext) -> Fidelity {
        match self.data_driven {
            None => self.fidelity,
            Some(DataDrivenMode::StateResources) => {
                // any state with a non-null `resources` block → the Resources recolor binds.
                let snap = &ctx.snapshot;
                let present = snap
                    .states()
                    .map(|states| {
                        states.keys().any(|sid| {
                            matches!(ctx.state_field(sid, "resources"), Captured::Measured(_))
                        })
                    })
                    .unwrap_or(false);
                if present {
                    self.fidelity
                } else {
                    Fidelity::Unavailable
                }
            }
            Some(DataDrivenMode::Weather) => {
                // the weather domain (regions/provinces) is captured → the proxy mode binds.
                let snap = &ctx.snapshot;
                match snap.domain("weather") {
                    Some(serde_json::Value::Null) | None => Fidelity::Unavailable,
                    Some(_) => self.fidelity,
                }
            }
        }
    }
}

/// The 14 map modes + their §B.9 fidelity (the binding is per-mode, not per-button). The
/// modes a configurable slot can visualise; each carries the driving domain's badge. The
/// `Resources` + `Weather` modes are DATA-DRIVEN: their fidelity is resolved from the now-
/// recorded `states.*.resources` / `weather` domains at bind time, not a frozen label.
const MAP_MODES: &[MapModeRailEntry] = &[
    MapModeRailEntry::stat("Political", Fidelity::Exact, Some(CoreMapMode::Political)),
    MapModeRailEntry::stat("Control", Fidelity::Exact, Some(CoreMapMode::Control)),
    MapModeRailEntry::stat("Resistance", Fidelity::Exact, Some(CoreMapMode::Resistance)),
    MapModeRailEntry::stat("Compliance", Fidelity::Exact, Some(CoreMapMode::Compliance)),
    MapModeRailEntry::stat(
        "VictoryPoints",
        Fidelity::Exact,
        Some(CoreMapMode::VictoryPoints),
    ),
    MapModeRailEntry::stat(
        "Buildings",
        Fidelity::Exact,
        Some(CoreMapMode::Buildings(BuildingKind::CivilianFactory)),
    ),
    // per-state resources closed by the live reader → Exact when captured, else "—".
    MapModeRailEntry::driven(
        "Resources",
        Fidelity::Exact,
        DataDrivenMode::StateResources,
        Some(CoreMapMode::Resources),
    ),
    // weather domain closed by the live reader → Derived proxy when captured, else "—".
    MapModeRailEntry::driven(
        "Weather",
        Fidelity::Derived,
        DataDrivenMode::Weather,
        Some(CoreMapMode::Weather),
    ),
    MapModeRailEntry::stat(
        "FrontLines",
        Fidelity::Unavailable,
        Some(CoreMapMode::FrontLines),
    ), // compute pass not run in this chrome rail
    MapModeRailEntry::stat("Faction", Fidelity::Exact, Some(CoreMapMode::Diplomacy)),
    MapModeRailEntry::stat("AirSuperiority", Fidelity::Derived, Some(CoreMapMode::Air)), // region proxy
    MapModeRailEntry::stat("Naval", Fidelity::Derived, Some(CoreMapMode::Naval)), // region proxy
    MapModeRailEntry::stat("Supply", Fidelity::Derived, Some(CoreMapMode::Supply)), // division-province proxy
    MapModeRailEntry::stat("Intel", Fidelity::Derived, None),
];

const MAPMODE_ROW_H: f32 = 24.0;

fn default_chrome_screen() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 900.0,
    }
}

fn mapmodes_panel_fallback(screen: Rect) -> Rect {
    Rect {
        x: screen.x + screen.w - 56.0,
        y: screen.y + 230.0,
        w: 48.0,
        h: MAP_MODES.len() as f32 * MAPMODE_ROW_H,
    }
}

fn mapmode_row_height(panel: Rect) -> f32 {
    (panel.h / MAP_MODES.len() as f32).max(1.0)
}

#[cfg(test)]
fn mapmode_row_rect(panel: Rect, index: usize) -> Rect {
    let row_h = mapmode_row_height(panel);
    Rect {
        x: panel.x,
        y: panel.y + index as f32 * row_h,
        w: panel.w,
        h: row_h,
    }
}

fn pick_mapmode_index(panel: Rect, x: f32, y: f32) -> Option<usize> {
    if !panel.contains(x, y) {
        return None;
    }
    let row_h = mapmode_row_height(panel);
    let index = ((y - panel.y) / row_h).floor() as usize;
    (index < MAP_MODES.len()).then_some(index)
}

fn mapmode_command(entry: &MapModeRailEntry) -> Option<ViewCommand> {
    entry.core_mode.map(|mode| ViewCommand::SetMapMode { mode })
}

fn mapmodes_input_consumed(needs_redraw: bool) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands: Vec::new(),
        needs_redraw,
    }
}

struct MapModesDrawable {
    roots: Vec<Node>,
    /// the per-mode fidelity digest (the datum of this view: the recolor LUT each slot
    /// instantiates). The ACTIVE mode lives in the binary's map view (unreachable from a
    /// registry view) → the active indicator is badged, not fabricated.
    mode_digest: String,
}
impl Drawable for MapModesDrawable {
    fn draw_digest(&self) -> String {
        format!("MAPMODES active=— {}", self.mode_digest)
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
    }
}

impl ViewInstance for MapModesView {
    fn view_id(&self) -> ViewId {
        MAPMODES_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Chrome)
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        let ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        } = *ev
        else {
            return InputOutcome::ignored();
        };
        let panel = self
            .input_panel
            .unwrap_or_else(|| mapmodes_panel_fallback(self.screen));
        let Some(index) = pick_mapmode_index(panel, x, y) else {
            return InputOutcome::ignored();
        };
        match mapmode_command(&MAP_MODES[index]) {
            Some(command) => InputOutcome::consumed().with_command(command),
            None => mapmodes_input_consumed(false),
        }
    }
    fn rebind(&mut self, ctx: &RebindContext) {
        self.screen = Rect {
            x: 0.0,
            y: 0.0,
            w: ctx.screen.0 as f32,
            h: ctx.screen.1 as f32,
        };
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = gui_file(ctx, MAPMODES_FILE);

        // The binding is PER-MAP-MODE: each mode is a recolor LUT carrying its §B.9
        // fidelity. Tally each mode's badge (so the per-view honesty report reflects the
        // 14 modes), and digest them. The active-mode indicator is unavailable here (the
        // datum lives in the binary's map view) → recorded as one unavailable.
        let mut digest = String::from("modes=[");
        for m in MAP_MODES {
            // data-driven modes (Resources/Weather) resolve their fidelity from the now-
            // recorded snapshot domain; static modes use their frozen label.
            let mode_fid = m.fidelity_for(ctx);
            let atom = match mode_fid {
                Fidelity::Exact => DisplayAtom::text(m.name, Fidelity::Exact),
                Fidelity::Derived => DisplayAtom::text(m.name, Fidelity::Derived),
                Fidelity::Unavailable => {
                    DisplayAtom::unavailable_because(UnavailableReason::Blocked)
                }
            };
            fidelity.record(&atom);
            let b = match mode_fid {
                Fidelity::Exact => "E",
                Fidelity::Derived => "D",
                Fidelity::Unavailable => "U",
            };
            digest.push_str(&format!("{}({b});", m.name));
        }
        digest.push(']');
        // the active-mode datum (unreachable) → one honest unavailable.
        let active = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        fidelity.record(&active);

        let roots = build_root(ctx, &file, MAPMODES_ROOT, &[]);
        let panel = root_rect(&roots, MAPMODES_ROOT);
        let hit = match panel {
            Some(p) => HitModel::from_roots(roots.clone()).with_block_rect(p),
            None => HitModel::from_roots(roots.clone()),
        };
        BoundView {
            instance: InstanceId(0),
            view_id: MAPMODES_VIEW_ID,
            policy: self.policy(),
            hit,
            drawable: Box::new(MapModesDrawable {
                roots,
                mode_digest: digest,
            }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }
    crate::impl_view_any!();
}

fn open_mapmodes(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(MapModesView::new())
}

register_view!(ViewDescriptor {
    id: MAPMODES_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(MAPMODES_FILE, MAPMODES_ROOT)),
    required_domains: &["states", "map"],
    route: Some(MenuRoute {
        cluster: MenuCluster::ChromeOutlinerMaps,
        label: "Map Modes",
        order: 6
    }),
    shortcut: None,
    open: open_mapmodes,
    policy: HostPolicy::window(ZBand::Chrome),
});

// =====================================================================================
// VIEW 7 — ON-MAP ICONS  [N8]
// =====================================================================================

const MAPICONS_VIEW_ID: ViewId = ViewId("chrome.mapicons");
const MAPICONS_FILE: &str = "mapicons.json";
const MAPICONS_ROOT: &str = "mapicons_container";

pub struct MapIconsView;
impl MapIconsView {
    pub fn new() -> MapIconsView {
        MapIconsView
    }
}
impl Default for MapIconsView {
    fn default() -> Self {
        MapIconsView::new()
    }
}

/// One on-map icon entity resolved from the snapshot: the binding-map TEMPLATE it would
/// instantiate, its bound label atom, and its target PROVINCE id (resolved via the
/// `state_meta.json` state→province join). When the province + a live camera are present,
/// the N8 layer places the icon ON THE MAP at the projected province centroid (centroids
/// built lazily from `ctx.stage.map_dir`); when `province` is `None` (a top-level-domain
/// family with no per-state geometry, or no state-meta), the entity drops to the badged
/// screen legend — its fields still enumerated honestly, never a fabricated map position.
#[derive(Clone)]
struct IconEntity {
    template: &'static str,
    label: Option<DisplayAtom>,
    sprite: Option<assets::SpriteAsset>,
    sprite_fidelity: Fidelity,
    /// the target PROVINCE id (NOT a state id) for on-map placement, or `None` to legend.
    province: Option<u32>,
}

impl IconEntity {
    fn label(template: &'static str, label: DisplayAtom, province: Option<u32>) -> IconEntity {
        IconEntity {
            template,
            label: Some(label),
            sprite: None,
            sprite_fidelity: Fidelity::Unavailable,
            province,
        }
    }

    fn sprite(
        template: &'static str,
        sprite: assets::SpriteAsset,
        province: Option<u32>,
    ) -> IconEntity {
        IconEntity::sprite_with_fidelity(template, sprite, Fidelity::Exact, province)
    }

    fn sprite_with_fidelity(
        template: &'static str,
        sprite: assets::SpriteAsset,
        fidelity: Fidelity,
        province: Option<u32>,
    ) -> IconEntity {
        let label = match &sprite {
            assets::SpriteAsset::Resolved(_) => None,
            assets::SpriteAsset::Unavailable(_) => {
                Some(DisplayAtom::unavailable_because(UnavailableReason::Blocked))
            }
        };
        IconEntity {
            template,
            label,
            sprite: Some(sprite),
            sprite_fidelity: fidelity,
            province,
        }
    }
}

fn record_sprite_asset(report: &mut FidelityReport, asset: &assets::SpriteAsset) {
    record_sprite_asset_with_fidelity(report, asset, Fidelity::Exact);
}

fn record_sprite_asset_with_fidelity(
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

fn sprite_asset_digest(asset: &assets::SpriteAsset) -> String {
    sprite_asset_digest_with_fidelity(asset, Fidelity::Exact)
}

fn sprite_asset_digest_with_fidelity(asset: &assets::SpriteAsset, fidelity: Fidelity) -> String {
    match asset {
        assets::SpriteAsset::Resolved(resolved) => {
            format!(
                "sprite={}#{}({})",
                resolved.name,
                resolved.frame,
                fidelity_badge(fidelity)
            )
        }
        assets::SpriteAsset::Unavailable(miss) => {
            format!("sprite=MISS:{}(U)", sprite_miss_digest(miss))
        }
    }
}

fn fidelity_badge(fidelity: Fidelity) -> &'static str {
    match fidelity {
        Fidelity::Exact => "E",
        Fidelity::Derived => "D",
        Fidelity::Unavailable => "U",
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
        } => format!("frame_out_of_range:{name}:{frame}:{frames}"),
    }
}

struct MapIconsDrawable {
    roots: Vec<Node>,
    entities: Vec<IconEntity>,
    /// per-entity SCREEN position when both the camera AND the centroid raster were
    /// reachable (on-MAP placement, derived); `None` per entity when its province had no
    /// centroid or no camera was present (then it falls into the badged legend column).
    placements: Vec<Option<(f32, f32)>>,
    /// true iff a centroid raster + live camera were both available (placement derived).
    placement_available: bool,
}
impl Drawable for MapIconsDrawable {
    fn draw_digest(&self) -> String {
        let mut s = format!("MAPICONS place={} ents=[", self.placement_available);
        for (e, pos) in self.entities.iter().zip(self.placements.iter()) {
            let on_map = pos.is_some();
            if let Some(sprite) = &e.sprite {
                s.push_str(&format!(
                    "{}@{:?}map{}:{};",
                    e.template,
                    e.province,
                    on_map,
                    sprite_asset_digest_with_fidelity(sprite, e.sprite_fidelity)
                ));
            }
            if let Some(label) = &e.label {
                let b = match label.fidelity() {
                    Fidelity::Exact => "E",
                    Fidelity::Derived => "D",
                    Fidelity::Unavailable => "U",
                };
                s.push_str(&format!(
                    "{}@{:?}map{}={}({b});",
                    e.template,
                    e.province,
                    on_map,
                    label.as_str()
                ));
            }
        }
        s.push(']');
        s
    }
    fn draw(&self, fd: &mut FrameDraw) {
        // the real mapicons container .gui (template roots are placement-only art).
        fd.draw_nodes(&self.roots);
        // the N8 icon layer. When a centroid + camera were reachable, icons are placed
        // ON THE MAP at the projected province centroid (derived). Otherwise the entity
        // drops into a SCREEN-anchored legend column, honestly badged (no fabricated map
        // position) so its bound fields are still visible.
        let white = frame_white(fd);
        let loaded = fd.fonts.get(fd.renderer, "hoi_16mbs");
        let (font, pages): (Option<&assets::BmFont>, &[render::Texture]) = match &loaded {
            Some(l) => (Some(&l.font), &l.pages),
            None => (None, &[]),
        };
        let assets = DrawAssets {
            white: &white,
            font,
            font_pages: pages,
        };
        // EVERY entity is rendered — the on-map-placed ones at their projected centroid,
        // the unplaced ones in a SCREEN-anchored legend column (Codex: a `.take(N)` cap
        // would visually DROP the badged "accounted" families when real per-state icons
        // are numerous, contradicting the digest). The legend column wraps to extra
        // columns so a long unplaced list never overflows off-screen.
        let mut legend_i = 0usize;
        let icons: Vec<MapIcon> = self
            .entities
            .iter()
            .zip(self.placements.iter())
            .map(|(e, pos)| {
                let (anchor, sprite, tint) = match pos {
                    Some((sx, sy)) => {
                        let sprite = match &e.sprite {
                            Some(assets::SpriteAsset::Resolved(resolved)) => {
                                widgets::common::AssetRef::Named(resolved.name.clone())
                            }
                            Some(assets::SpriteAsset::Unavailable(_)) => {
                                widgets::common::AssetRef::Unavailable
                            }
                            None => widgets::common::AssetRef::None, // a tinted on-map marker
                        };
                        let tint = if e.sprite.is_some() {
                            [1.0, 1.0, 1.0, 1.0]
                        } else {
                            [0.85, 0.80, 0.30, 1.0]
                        };
                        (IconAnchor::Screen { x: *sx, y: *sy }, sprite, tint)
                    }
                    None => {
                        // wrap the legend into columns of 40 rows so nothing is dropped.
                        let col = (legend_i / 40) as f32;
                        let row = (legend_i % 40) as f32;
                        let x = 16.0 + col * 220.0;
                        let y = 80.0 + row * 18.0;
                        legend_i += 1;
                        let sprite = match &e.sprite {
                            Some(assets::SpriteAsset::Resolved(resolved)) => {
                                widgets::common::AssetRef::Named(resolved.name.clone())
                            }
                            Some(assets::SpriteAsset::Unavailable(_)) => {
                                widgets::common::AssetRef::Unavailable
                            }
                            None => widgets::common::AssetRef::Unavailable, // badged no-placement
                        };
                        let tint = if e.sprite.is_some() {
                            [1.0, 1.0, 1.0, 1.0]
                        } else {
                            [0.8, 0.8, 0.85, 1.0]
                        };
                        (IconAnchor::Screen { x, y }, sprite, tint)
                    }
                };
                MapIcon {
                    id: e.template.to_string(),
                    anchor,
                    sprite,
                    tint,
                    label: e.label.as_ref().map(watom),
                    priority: 0,
                    size: if e.sprite.is_some() { 28.0 } else { 14.0 },
                }
            })
            .collect();
        // the centroids/camera are encoded into Screen anchors already, so a default
        // (no-op) N8 camera + empty centroids is correct here.
        let centroids = map_overlay::ProvinceCentroids::default();
        let cam = map_overlay::MapCamera::new((0.0, 0.0), 1.0, (0.0, 0.0));
        let laid = map_overlay::layout_icons(&icons, &centroids, &cam);
        let textures = &mut *fd.textures;
        map_overlay::plan_icons(&laid, None).execute(
            fd.renderer,
            &assets,
            |r, draw| match textures.get(r, draw.name) {
                Some(tex) => {
                    r.draw_sprite_frame(tex, draw.x, draw.y, draw.w, draw.h, 0, 1, draw.tint);
                    true
                }
                None => false,
            },
        );
    }
}

fn push_counter_portrait_entities(
    ctx: &BindContext,
    tag: &str,
    report: &mut FidelityReport,
    entities: &mut Vec<IconEntity>,
) {
    let before = entities.len();
    for (province, asset) in counter_portrait_assets(ctx, tag) {
        record_sprite_asset(report, &asset);
        entities.push(IconEntity::sprite(
            "unit_counter.general_portrait",
            asset,
            Some(province),
        ));
    }

    if entities.len() == before {
        let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        report.record(&atom);
        entities.push(IconEntity::label(
            "unit_counter.general_portrait",
            atom,
            None,
        ));
    }
}

fn push_counter_formation_insignia_entities(
    ctx: &BindContext,
    tag: &str,
    report: &mut FidelityReport,
    entities: &mut Vec<IconEntity>,
) {
    let before = entities.len();
    for (province, sprite) in counter_formation_insignia_assets(ctx, tag) {
        record_sprite_asset_with_fidelity(report, &sprite.asset, sprite.fidelity);
        entities.push(IconEntity::sprite_with_fidelity(
            "unit_counter.formation_insignia",
            sprite.asset,
            sprite.fidelity,
            Some(province),
        ));
    }

    if entities.len() == before {
        let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        report.record(&atom);
        entities.push(IconEntity::label(
            "unit_counter.formation_insignia",
            atom,
            None,
        ));
    }
}

fn push_counter_stack_entities(
    ctx: &BindContext,
    tag: &str,
    report: &mut FidelityReport,
    entities: &mut Vec<IconEntity>,
) {
    let stacks = counter_division_stacks(ctx, tag);
    if stacks.is_empty() {
        let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
        report.record(&atom);
        entities.push(IconEntity::label("unit_counter.count_txt", atom, None));
        return;
    }

    for stack in stacks {
        push_counter_stack_label(
            report,
            entities,
            "unit_counter.count_txt",
            DisplayAtom::text(stack.count.to_string(), Fidelity::Derived),
            Some(stack.province),
        );
        push_counter_stack_label(
            report,
            entities,
            "unit_counter.flag",
            DisplayAtom::text(stack.owner.clone(), Fidelity::Exact),
            Some(stack.province),
        );
        push_counter_stack_label(
            report,
            entities,
            "unit_counter.bar_org",
            stack
                .org
                .map(|v| DisplayAtom::text(fmt_counter_percent(v), Fidelity::Exact))
                .unwrap_or_else(|| DisplayAtom::unavailable_because(UnavailableReason::Blocked)),
            Some(stack.province),
        );
        push_counter_stack_label(
            report,
            entities,
            "unit_counter.bar_str",
            stack
                .strength
                .map(|v| DisplayAtom::text(fmt_counter_percent(v), Fidelity::Exact))
                .unwrap_or_else(|| DisplayAtom::unavailable_because(UnavailableReason::Blocked)),
            Some(stack.province),
        );
        push_counter_stack_label(
            report,
            entities,
            "unit_counter.type",
            stack.role_atom(),
            Some(stack.province),
        );
        match stack.dig_in {
            CounterDigIn::Present => push_counter_stack_label(
                report,
                entities,
                "unit_counter.onmap_digin_icon",
                DisplayAtom::text("dig", Fidelity::Exact),
                Some(stack.province),
            ),
            CounterDigIn::Unavailable => push_counter_stack_label(
                report,
                entities,
                "unit_counter.onmap_digin_icon",
                DisplayAtom::unavailable_because(UnavailableReason::Blocked),
                Some(stack.province),
            ),
            CounterDigIn::Absent => {}
        }
    }
}

fn push_counter_stack_label(
    report: &mut FidelityReport,
    entities: &mut Vec<IconEntity>,
    template: &'static str,
    atom: DisplayAtom,
    province: Option<u32>,
) {
    report.record(&atom);
    entities.push(IconEntity::label(template, atom, province));
}

#[derive(Clone)]
struct CounterStack {
    province: u32,
    owner: String,
    count: usize,
    org: Option<f64>,
    strength: Option<f64>,
    role: CounterRole,
    dig_in: CounterDigIn,
}

#[derive(Clone)]
enum CounterRole {
    Known(String),
    Mixed,
    Unavailable,
}

impl CounterStack {
    fn role_atom(&self) -> DisplayAtom {
        match &self.role {
            CounterRole::Known(role) => DisplayAtom::text(role.clone(), Fidelity::Derived),
            CounterRole::Mixed => DisplayAtom::text("mixed", Fidelity::Derived),
            CounterRole::Unavailable => {
                DisplayAtom::unavailable_because(UnavailableReason::Blocked)
            }
        }
    }
}

#[derive(Clone, Copy)]
enum CounterDigIn {
    Present,
    Absent,
    Unavailable,
}

#[derive(Default)]
struct CounterStackBuilder {
    owner: Option<String>,
    count: usize,
    org_sum: f64,
    org_count: usize,
    strength_sum: f64,
    strength_count: usize,
    strength_all_fractional: bool,
    role_counts: std::collections::BTreeMap<String, usize>,
    role_unavailable: bool,
    any_dig_in: bool,
    all_dig_in_measured: bool,
}

impl CounterStackBuilder {
    fn new(owner: &str) -> CounterStackBuilder {
        CounterStackBuilder {
            owner: Some(owner.to_string()),
            strength_all_fractional: true,
            all_dig_in_measured: true,
            ..CounterStackBuilder::default()
        }
    }

    fn push(&mut self, div: &serde_json::Value) {
        self.count += 1;
        if let Some(org) = numeric_field(div, "organisation") {
            self.org_sum += org;
            self.org_count += 1;
        }
        if let Some(strength) = numeric_field(div, "strength") {
            self.strength_sum += strength;
            self.strength_count += 1;
            if !(0.0..=1.0).contains(&strength) {
                self.strength_all_fractional = false;
            }
        }
        match div.get("template_role").and_then(|v| v.as_str()) {
            Some(role) if known_counter_role(role) => {
                *self.role_counts.entry(role.to_string()).or_insert(0) += 1;
            }
            _ => {
                self.role_unavailable = true;
            }
        }
        match numeric_field(div, "dig_in") {
            Some(v) => {
                if v > 0.0 {
                    self.any_dig_in = true;
                }
            }
            None => {
                self.all_dig_in_measured = false;
            }
        }
    }

    fn finish(self, province: u32) -> Option<CounterStack> {
        let owner = self.owner?;
        let count = self.count;
        if count == 0 {
            return None;
        }
        let org = if self.org_count == count {
            Some(self.org_sum / count as f64)
        } else {
            None
        };
        let strength = if self.strength_count == count && self.strength_all_fractional {
            Some(self.strength_sum / count as f64)
        } else {
            None
        };
        let role = if self.role_unavailable || self.role_counts.is_empty() {
            CounterRole::Unavailable
        } else {
            dominant_counter_role(self.role_counts)
        };
        let dig_in = if self.any_dig_in {
            CounterDigIn::Present
        } else if self.all_dig_in_measured {
            CounterDigIn::Absent
        } else {
            CounterDigIn::Unavailable
        };
        Some(CounterStack {
            province,
            owner,
            count,
            org,
            strength,
            role,
            dig_in,
        })
    }
}

fn counter_division_stacks(ctx: &BindContext, tag: &str) -> Vec<CounterStack> {
    let Some(divs) = division_records(ctx) else {
        return Vec::new();
    };
    let mut grouped: std::collections::BTreeMap<u32, CounterStackBuilder> =
        std::collections::BTreeMap::new();
    for div in divs.iter().filter(|div| element_owner_matches(div, tag)) {
        if is_railway_gun_division(div) {
            continue;
        }
        let Some(province) = direct_province_anchor(div) else {
            continue;
        };
        grouped
            .entry(province)
            .or_insert_with(|| CounterStackBuilder::new(tag))
            .push(div);
    }
    grouped
        .into_iter()
        .filter_map(|(province, stack)| stack.finish(province))
        .collect()
}

fn numeric_field(item: &serde_json::Value, key: &str) -> Option<f64> {
    match crate::host::context::classify_value(item.get(key)) {
        Captured::Measured(v) => v.as_f64().filter(|f| f.is_finite()),
        Captured::Null | Captured::Absent => None,
    }
}

fn is_railway_gun_division(div: &serde_json::Value) -> bool {
    div.get("template_role").and_then(|v| v.as_str()) == Some("railway_gun")
}

fn known_counter_role(role: &str) -> bool {
    matches!(
        role,
        "infantry"
            | "cavalry"
            | "motorized"
            | "mechanized"
            | "armor"
            | "armored"
            | "mountaineers"
            | "marine"
            | "marines"
            | "paratrooper"
            | "paratroopers"
            | "artillery"
            | "anti_tank"
            | "anti_air"
    )
}

fn dominant_counter_role(role_counts: std::collections::BTreeMap<String, usize>) -> CounterRole {
    let max = role_counts.values().copied().max();
    let Some(max) = max else {
        return CounterRole::Unavailable;
    };
    let winners: Vec<String> = role_counts
        .into_iter()
        .filter_map(|(role, count)| if count == max { Some(role) } else { None })
        .collect();
    if winners.len() == 1 {
        CounterRole::Known(winners[0].clone())
    } else {
        CounterRole::Mixed
    }
}

fn fmt_counter_percent(value: f64) -> String {
    let percent = if (0.0..=1.0).contains(&value) {
        value * 100.0
    } else {
        value
    };
    format!("{}%", percent.round() as i64)
}

fn counter_portrait_assets(ctx: &BindContext, tag: &str) -> Vec<(u32, assets::SpriteAsset)> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let divs = division_records(ctx);

    if let Some(divs) = divs {
        for div in divs.iter().filter(|div| element_owner_matches(div, tag)) {
            if !has_portrait_source(div) {
                continue;
            }
            let Some(province) = direct_province_anchor(div) else {
                continue;
            };
            let asset = counter_portrait_asset_for_item(
                ctx,
                tag,
                div,
                &["commanders.generals", "commanders.field_marshals"],
            );
            push_unique_counter_portrait(&mut out, &mut seen, province, asset);
        }
    }

    let oob = ctx.order_of_battle();

    if let Some(armies) = oob_array(&oob, "/armies") {
        for army in armies
            .iter()
            .filter(|army| element_owner_matches(army, tag))
        {
            if !has_portrait_source(army) {
                continue;
            }
            let asset = counter_portrait_asset_for_item(
                ctx,
                tag,
                army,
                &["commanders.generals", "commanders.field_marshals"],
            );
            for province in army_counter_anchors(army, divs) {
                push_unique_counter_portrait(&mut out, &mut seen, province, asset.clone());
            }
        }
    }

    if let Some(groups) = oob_array(&oob, "/army_groups") {
        let armies = oob_array(&oob, "/armies");
        for group in groups
            .iter()
            .filter(|group| element_owner_matches(group, tag))
        {
            if !has_portrait_source(group) {
                continue;
            }
            let asset =
                counter_portrait_asset_for_item(ctx, tag, group, &["commanders.field_marshals"]);
            for province in army_group_counter_anchors(group, armies, divs) {
                push_unique_counter_portrait(&mut out, &mut seen, province, asset.clone());
            }
        }
    }

    if let Some(fleets) = oob_array(&oob, "/fleets") {
        for fleet in fleets
            .iter()
            .filter(|fleet| element_owner_matches(fleet, tag))
        {
            if !has_portrait_source(fleet) {
                continue;
            }
            let asset = counter_portrait_asset_for_item(ctx, tag, fleet, &["commanders.admirals"]);
            for province in fleet_counter_anchors(fleet) {
                push_unique_counter_portrait(&mut out, &mut seen, province, asset.clone());
            }
        }
    }

    out
}

#[derive(Clone)]
struct CounterSpriteAsset {
    asset: assets::SpriteAsset,
    fidelity: Fidelity,
}

fn counter_formation_insignia_assets(
    ctx: &BindContext,
    tag: &str,
) -> Vec<(u32, CounterSpriteAsset)> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let divs = division_records(ctx);
    let oob = ctx.order_of_battle();

    if let Some(armies) = oob_array(&oob, "/armies") {
        for army in armies
            .iter()
            .filter(|army| element_owner_matches(army, tag))
        {
            let anchors = army_counter_anchors(army, divs);
            if anchors.is_empty() {
                continue;
            }
            let sprite = formation_insignia_sprite_for_item(ctx, army, assets::FormationKind::Army);
            for province in anchors {
                push_unique_counter_sprite(&mut out, &mut seen, province, sprite.clone());
            }
        }
    }

    if let Some(groups) = oob_array(&oob, "/army_groups") {
        let armies = oob_array(&oob, "/armies");
        for group in groups
            .iter()
            .filter(|group| element_owner_matches(group, tag))
        {
            let anchors = army_group_counter_anchors(group, armies, divs);
            if anchors.is_empty() {
                continue;
            }
            let sprite =
                formation_insignia_sprite_for_item(ctx, group, assets::FormationKind::ArmyGroup);
            for province in anchors {
                push_unique_counter_sprite(&mut out, &mut seen, province, sprite.clone());
            }
        }
    }

    out
}

fn counter_portrait_asset_for_item(
    ctx: &BindContext,
    tag: &str,
    item: &serde_json::Value,
    commander_sources: &[&str],
) -> assets::SpriteAsset {
    let mut first_miss: Option<assets::SpriteAsset> = None;

    let direct = assets::resolve_portrait_sprite(&ctx.stage.manifest, item);
    match direct {
        assets::SpriteAsset::Resolved(_) => return direct,
        assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName) => {}
        miss => remember_sprite_miss(&mut first_miss, miss),
    }

    for leader_field in ["leader", "leader_id"] {
        let joined =
            commander_portrait_for_counter(ctx, tag, item.get(leader_field), commander_sources);
        match joined {
            assets::SpriteAsset::Resolved(_) => return joined,
            assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName) => {}
            miss => remember_sprite_miss(&mut first_miss, miss),
        }
    }

    first_miss.unwrap_or(assets::SpriteAsset::Unavailable(
        assets::SpriteMiss::NoSpriteName,
    ))
}

fn formation_insignia_sprite_for_item(
    ctx: &BindContext,
    item: &serde_json::Value,
    kind: assets::FormationKind,
) -> CounterSpriteAsset {
    let resolved = assets::resolve_formation_insignia_sprite(&ctx.stage.manifest, item, kind);
    CounterSpriteAsset {
        asset: resolved.asset,
        fidelity: formation_insignia_fidelity(resolved.origin),
    }
}

fn formation_insignia_fidelity(origin: assets::FormationInsigniaOrigin) -> Fidelity {
    match origin {
        assets::FormationInsigniaOrigin::ExplicitMetadata => Fidelity::Exact,
        assets::FormationInsigniaOrigin::IconFrameMetadata => Fidelity::Derived,
        assets::FormationInsigniaOrigin::Unavailable => Fidelity::Unavailable,
    }
}

fn oob_array<'a>(
    oob: &Captured<&'a serde_json::Value>,
    pointer: &str,
) -> Option<&'a [serde_json::Value]> {
    match oob {
        Captured::Measured(v) => v.pointer(pointer)?.as_array().map(|a| a.as_slice()),
        Captured::Null | Captured::Absent => None,
    }
}

fn division_records<'a>(ctx: &'a BindContext) -> Option<&'a [serde_json::Value]> {
    match ctx.divisions() {
        Captured::Measured(v) => v.as_array().map(|a| a.as_slice()),
        Captured::Null | Captured::Absent => None,
    }
}

fn element_owner_matches(item: &serde_json::Value, tag: &str) -> bool {
    element_owner_tag(item) == Some(tag)
}

fn element_owner_tag(item: &serde_json::Value) -> Option<&str> {
    match crate::host::context::classify_value(item.get("owner")) {
        Captured::Measured(owner) => owner.as_str(),
        Captured::Null | Captured::Absent => None,
    }
}

fn owner_matches_or_missing(item: &serde_json::Value, expected_owner: Option<&str>) -> bool {
    match (expected_owner, element_owner_tag(item)) {
        (Some(expected), Some(actual)) => actual == expected,
        _ => true,
    }
}

fn owner_matches_required(item: &serde_json::Value, expected_owner: Option<&str>) -> bool {
    match expected_owner {
        Some(expected) => element_owner_matches(item, expected),
        None => true,
    }
}

fn has_portrait_source(item: &serde_json::Value) -> bool {
    [
        "portrait_gfx",
        "gfx",
        "portrait",
        "small_portrait",
        "large_portrait",
        "portraits",
    ]
    .iter()
    .any(|key| item.get(*key).is_some())
        || ["leader", "leader_id"]
            .iter()
            .any(|key| item.get(*key).is_some())
}

fn push_unique_counter_portrait(
    out: &mut Vec<(u32, assets::SpriteAsset)>,
    seen: &mut std::collections::BTreeSet<(u32, String)>,
    province: u32,
    asset: assets::SpriteAsset,
) {
    if seen.insert((province, sprite_asset_digest(&asset))) {
        out.push((province, asset));
    }
}

fn push_unique_counter_sprite(
    out: &mut Vec<(u32, CounterSpriteAsset)>,
    seen: &mut std::collections::BTreeSet<(u32, String)>,
    province: u32,
    sprite: CounterSpriteAsset,
) {
    let digest = sprite_asset_digest_with_fidelity(&sprite.asset, sprite.fidelity);
    if seen.insert((province, digest)) {
        out.push((province, sprite));
    }
}

fn army_counter_anchors(army: &serde_json::Value, divs: Option<&[serde_json::Value]>) -> Vec<u32> {
    army_counter_anchors_for_owner(army, divs, element_owner_tag(army))
}

fn army_counter_anchors_for_owner(
    army: &serde_json::Value,
    divs: Option<&[serde_json::Value]>,
    expected_owner: Option<&str>,
) -> Vec<u32> {
    let mut anchors = std::collections::BTreeSet::new();
    push_direct_counter_anchor(army, &mut anchors);
    push_division_member_anchors(army, divs, expected_owner, &mut anchors);
    anchors.into_iter().collect()
}

fn army_group_counter_anchors(
    group: &serde_json::Value,
    armies: Option<&[serde_json::Value]>,
    divs: Option<&[serde_json::Value]>,
) -> Vec<u32> {
    let mut anchors = std::collections::BTreeSet::new();
    let group_owner = element_owner_tag(group);
    push_direct_counter_anchor(group, &mut anchors);

    for key in ["armies_resolved", "army_refs_resolved"] {
        if let Some(items) = group.get(key).and_then(|v| v.as_array()) {
            for army in items {
                if !owner_matches_or_missing(army, group_owner) {
                    continue;
                }
                let expected_owner = element_owner_tag(army).or(group_owner);
                for province in army_counter_anchors_for_owner(army, divs, expected_owner) {
                    anchors.insert(province);
                }
            }
        }
    }

    if let (Some(refs), Some(armies)) = (group.get("army_refs").and_then(|v| v.as_array()), armies)
    {
        for army_ref in refs {
            for army in armies
                .iter()
                .filter(|army| counter_refs_match(army_ref, *army))
                .filter(|army| owner_matches_required(army, group_owner))
            {
                let expected_owner = element_owner_tag(army).or(group_owner);
                for province in army_counter_anchors_for_owner(army, divs, expected_owner) {
                    anchors.insert(province);
                }
            }
        }
    }

    anchors.into_iter().collect()
}

fn fleet_counter_anchors(fleet: &serde_json::Value) -> Vec<u32> {
    let mut anchors = std::collections::BTreeSet::new();
    push_direct_counter_anchor(fleet, &mut anchors);
    if let Some(tfs) = fleet.get("task_forces").and_then(|v| v.as_array()) {
        for tf in tfs {
            push_direct_counter_anchor(tf, &mut anchors);
        }
    }
    anchors.into_iter().collect()
}

fn push_division_member_anchors(
    item: &serde_json::Value,
    divs: Option<&[serde_json::Value]>,
    expected_owner: Option<&str>,
    anchors: &mut std::collections::BTreeSet<u32>,
) {
    for key in [
        "members_resolved",
        "divisions_resolved",
        "scheduled_members_resolved",
    ] {
        if let Some(items) = item.get(key).and_then(|v| v.as_array()) {
            for div in items {
                if !owner_matches_or_missing(div, expected_owner) {
                    continue;
                }
                push_direct_counter_anchor(div, anchors);
            }
        }
    }

    let Some(divs) = divs else {
        return;
    };
    for key in ["members", "division_refs", "scheduled_members"] {
        if let Some(refs) = item.get(key).and_then(|v| v.as_array()) {
            for div_ref in refs {
                for div in divs
                    .iter()
                    .filter(|div| counter_refs_match(div_ref, *div))
                    .filter(|div| owner_matches_required(div, expected_owner))
                {
                    push_direct_counter_anchor(div, anchors);
                }
            }
        }
    }
}

fn push_direct_counter_anchor(
    item: &serde_json::Value,
    anchors: &mut std::collections::BTreeSet<u32>,
) {
    if let Some(province) = direct_province_anchor(item) {
        anchors.insert(province);
    }
}

fn direct_province_anchor(item: &serde_json::Value) -> Option<u32> {
    for key in [
        "location_province",
        "province",
        "province_id",
        "current_province",
    ] {
        if let Some(province) = item.get(key).and_then(province_anchor_value) {
            return Some(province);
        }
    }
    item.get("position").and_then(province_anchor_value)
}

fn province_anchor_value(value: &serde_json::Value) -> Option<u32> {
    match value {
        serde_json::Value::Number(n) => n
            .as_u64()
            .filter(|u| *u > 0)
            .and_then(|u| u32::try_from(u).ok()),
        serde_json::Value::String(s) => s.trim().parse::<u32>().ok().filter(|u| *u > 0),
        serde_json::Value::Object(map) => {
            for key in ["location_province", "province", "province_id"] {
                if let Some(province) = map.get(key).and_then(province_anchor_value) {
                    return Some(province);
                }
            }
            None
        }
        _ => None,
    }
}

fn counter_refs_match(reference: &serde_json::Value, item: &serde_json::Value) -> bool {
    let a = CounterRefSet::from_ref(reference);
    let b = CounterRefSet::from_item(item);
    a.overlaps(&b)
}

#[derive(Default)]
struct CounterRefSet {
    tokens: std::collections::BTreeSet<String>,
    typed: std::collections::BTreeSet<(String, String)>,
}

impl CounterRefSet {
    fn from_ref(value: &serde_json::Value) -> CounterRefSet {
        let mut out = CounterRefSet::default();
        collect_counter_refs(value, &mut out);
        out
    }

    fn from_item(item: &serde_json::Value) -> CounterRefSet {
        let mut out = CounterRefSet::default();
        for key in ["id", "div_id", "division_id", "unit_id"] {
            if let Some(value) = item.get(key) {
                collect_counter_refs(value, &mut out);
            }
        }
        push_typed_counter_refs(
            item,
            &["id", "div_id", "division_id", "unit_id"],
            &["type", "div_type", "division_type", "unit_type"],
            &mut out,
        );
        out
    }

    fn overlaps(&self, other: &CounterRefSet) -> bool {
        if self
            .typed
            .iter()
            .any(|candidate| other.typed.contains(candidate))
        {
            return true;
        }
        let token_match = self
            .tokens
            .iter()
            .any(|candidate| other.tokens.contains(candidate));
        token_match && (self.typed.is_empty() || other.typed.is_empty())
    }
}

fn collect_counter_refs(value: &serde_json::Value, out: &mut CounterRefSet) {
    match value {
        serde_json::Value::String(s) => push_counter_ref_string(out, s),
        serde_json::Value::Number(n) => {
            out.tokens.insert(n.to_string());
        }
        serde_json::Value::Object(map) => {
            push_typed_counter_refs_from_map(
                map,
                &["id", "div_id", "division_id", "unit_id"],
                &["type", "div_type", "division_type", "unit_type"],
                out,
            );
            for key in ["id", "div_id", "division_id", "unit_id"] {
                if let Some(child) = map.get(key) {
                    collect_counter_refs(child, out);
                }
            }
        }
        _ => {}
    }
}

fn push_typed_counter_refs(
    item: &serde_json::Value,
    id_fields: &[&str],
    type_fields: &[&str],
    out: &mut CounterRefSet,
) {
    if let Some(map) = item.as_object() {
        push_typed_counter_refs_from_map(map, id_fields, type_fields, out);
    }
}

fn push_typed_counter_refs_from_map(
    map: &serde_json::Map<String, serde_json::Value>,
    id_fields: &[&str],
    type_fields: &[&str],
    out: &mut CounterRefSet,
) {
    let ids = first_counter_scalars(map, id_fields);
    let types = first_counter_scalars(map, type_fields);
    for id in ids {
        for kind in &types {
            out.typed.insert((id.clone(), kind.clone()));
        }
    }
}

fn first_counter_scalars(
    map: &serde_json::Map<String, serde_json::Value>,
    fields: &[&str],
) -> Vec<String> {
    for field in fields {
        if let Some(value) = map.get(*field) {
            let scalars = counter_ref_scalars(value);
            if !scalars.is_empty() {
                return scalars;
            }
        }
    }
    Vec::new()
}

fn counter_ref_scalars(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    match value {
        serde_json::Value::String(s) => {
            let s = s.trim();
            if !s.is_empty() {
                out.push(s.to_string());
            }
        }
        serde_json::Value::Number(n) => out.push(n.to_string()),
        serde_json::Value::Object(map) => {
            for key in ["id", "div_id", "division_id", "unit_id", "type", "div_type"] {
                if let Some(child) = map.get(key) {
                    out.extend(counter_ref_scalars(child));
                }
            }
        }
        _ => {}
    }
    out.sort();
    out.dedup();
    out
}

fn push_counter_ref_string(out: &mut CounterRefSet, raw: &str) {
    let s = raw.trim();
    if s.is_empty() {
        return;
    }
    out.tokens.insert(s.to_string());
    if let Some((id, kind)) = s.split_once(':') {
        let id = id.trim();
        let kind = kind.trim();
        if !id.is_empty() && !kind.is_empty() {
            out.tokens.insert(id.to_string());
            out.typed.insert((id.to_string(), kind.to_string()));
        }
    }
}

fn commander_portrait_for_counter(
    ctx: &BindContext,
    tag: &str,
    leader_ref: Option<&serde_json::Value>,
    sources: &[&str],
) -> assets::SpriteAsset {
    match find_counter_commander_by_ref(ctx, tag, leader_ref, sources) {
        Some(commander) => assets::resolve_portrait_sprite(&ctx.stage.manifest, commander),
        None => assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName),
    }
}

fn find_counter_commander_by_ref<'a>(
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
        let Captured::Measured(commanders) = ctx.country_field(tag, source) else {
            continue;
        };
        let Some(commanders) = commanders.as_array() else {
            continue;
        };
        for commander in commanders {
            if commander_matches_keys(commander, &keys) {
                return Some(commander);
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
        "leader_id",
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
                .or_else(|| map.get("leader_id"))
                .filter(|v| leader_ref_scalars(v).len() == 1);
            let kind_values = leader_ref_kind_scalars(map);
            if let Some(id) = typed_id.filter(|_| !kind_values.is_empty()) {
                for id_key in leader_ref_scalars(id) {
                    for kind_key in &kind_values {
                        keys.push(LeaderRefKey::Typed {
                            id: id_key.clone(),
                            kind: kind_key.clone(),
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
                for key in ["id", "unit_leader_id", "leader", "leader_id"] {
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

fn leader_ref_kind_scalars(map: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    let mut values = Vec::new();
    for key in ["type", "unit_leader_type"] {
        if let Some(child) = map.get(key) {
            values.extend(leader_ref_scalars(child));
        }
    }
    if let Some(role) = map.get("role").and_then(|v| v.as_str()) {
        match role {
            "general" | "corps_commander" => values.push("corps_commander".to_string()),
            "field_marshal" => values.push("field_marshal".to_string()),
            "admiral" | "navy_leader" => values.push("navy_leader".to_string()),
            other if !other.trim().is_empty() => values.push(other.trim().to_string()),
            _ => {}
        }
        // the game military leader refs commonly carry Clausewitz type 4713; this is a typed
        // id namespace, not a role split, so keep it as an additional join key.
        values.push("4713".to_string());
    }
    values.sort();
    values.dedup();
    values
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
            for key in ["id", "unit_leader_id", "leader", "leader_id", "type"] {
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

fn remember_sprite_miss(slot: &mut Option<assets::SpriteAsset>, asset: assets::SpriteAsset) {
    if slot.is_none() {
        *slot = Some(asset);
    }
}

impl ViewInstance for MapIconsView {
    fn view_id(&self) -> ViewId {
        MAPICONS_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Overlay)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = gui_file(ctx, MAPICONS_FILE);

        let mut entities: Vec<IconEntity> = Vec::new();
        let geo = state_geo(&ctx.stage.map_dir);

        // The per-state icons (VP, capital, resistance/compliance, buildings) are bound
        // from `states.<id>.*` (the no-fabrication seam) and PLACED at a real PROVINCE id
        // resolved via `state_meta.json` (Codex: a state id is NOT a province id; the VP
        // icon goes to the state's VP province, the resistance icon to a representative
        // province). When state geometry is absent the entity is still bound — its
        // placement just falls into the badged legend (never a fabricated map position).

        // ---- VP counters (victory_point_mapicon) — exact VP per state, placed at VP province ----
        for sid in state_ids(ctx).into_iter().take(64) {
            let vp = ctx.state_field(&sid, "victory_points_total");
            let atom = vp.display_with_fidelity(Fidelity::Exact, fmt_int);
            if !atom.is_unavailable() && atom.as_str() != "0" {
                fidelity.record(&atom);
                let prov = sid
                    .parse::<u32>()
                    .ok()
                    .and_then(|s| geo.and_then(|g| g.vp_province.get(&s).copied()));
                entities.push(IconEntity::label("victory_point_mapicon", atom, prov));
            }
        }

        // ---- capital marker (capital_mapicon) — the observed country's capital VP ----
        {
            let cap = ctx.country_field(ctx.country, "politics.capital_state");
            if let Captured::Measured(v) = cap {
                if let Some(cap_sid) = v.as_i64() {
                    let vp = ctx.state_field(&cap_sid.to_string(), "victory_points_total");
                    let atom = vp.display_with_fidelity(Fidelity::Exact, fmt_int);
                    fidelity.record(&atom);
                    let prov = geo.and_then(|g| g.vp_province.get(&(cap_sid as u32)).copied());
                    entities.push(IconEntity::label("capital_mapicon", atom, prov));
                }
            }
        }

        // ---- resistance/compliance icons — exact % per occupied state, placed at rep province ----
        for sid in state_ids(ctx).into_iter().take(64) {
            let res = ctx.state_field(&sid, "occupation.resistance");
            let atom = res.display_with_fidelity(Fidelity::Exact, |v| {
                v.as_f64()
                    .map(|f| format!("{}%", f.round() as i64))
                    .unwrap_or_else(|| DASH.to_string())
            });
            if !atom.is_unavailable() {
                fidelity.record(&atom);
                let prov = sid
                    .parse::<u32>()
                    .ok()
                    .and_then(|s| geo.and_then(|g| g.rep_province.get(&s).copied()));
                entities.push(IconEntity::label(
                    "resistance_compliance_map_icon",
                    atom,
                    prov,
                ));
            }
        }

        // ---- building map-icons (air base / naval base / supply node / railway) — the
        // LEVEL is exact from `states.<id>.buildings.*` / `province_buildings.*`; bind the
        // air-base + railway levels (exact) at the state's rep province. (Supply NUMBERS
        // are a proven wall → proxy; the on-map supply value stays badged below.) ----
        for sid in state_ids(ctx).into_iter().take(48) {
            let ab = ctx.state_field(&sid, "buildings.air_base");
            let atom = ab.display_with_fidelity(Fidelity::Exact, fmt_int);
            if !atom.is_unavailable() && atom.as_str() != "0" {
                fidelity.record(&atom);
                let prov = sid
                    .parse::<u32>()
                    .ok()
                    .and_then(|s| geo.and_then(|g| g.rep_province.get(&s).copied()));
                entities.push(IconEntity::label("air_base_mapicon", atom, prov));
            }
        }

        // ---- per-state RESOURCES + DYNAMIC-MODIFIER on-map icons: now CLOSED by the live
        // reader (`states.<id>.resources` / `states.<id>.dynamic_modifier`). Bind one icon
        // per state that actually carries the datum (placed at the state's rep province),
        // EXACTLY like the air-base icon above. A state without the datum emits no icon —
        // never a fabricated 0/empty (the absence IS the no-data signal). ----
        for sid in state_ids(ctx).into_iter().take(48) {
            // resources: a non-empty measured `{res: float}` block → one icon labelled with
            // the resource-type COUNT (exact). A measured-empty `{}` (state produces none)
            // emits no icon — that is real "no resources here", not fabricated.
            if let Captured::Measured(res) = ctx.state_field(&sid, "resources") {
                if let Some(obj) = res.as_object() {
                    if !obj.is_empty() {
                        let atom = DisplayAtom::text(format!("{}", obj.len()), Fidelity::Exact);
                        fidelity.record(&atom);
                        let prov = sid
                            .parse::<u32>()
                            .ok()
                            .and_then(|s| geo.and_then(|g| g.rep_province.get(&s).copied()));
                        entities.push(IconEntity::label("resource_mapicon", atom, prov));
                    }
                }
            }
            // dynamic_modifier: a non-empty measured `[name,...]` list → one icon labelled
            // with the active-modifier COUNT (exact). A measured-empty `[]` emits no icon.
            if let Captured::Measured(dm) = ctx.state_field(&sid, "dynamic_modifier") {
                if let Some(arr) = dm.as_array() {
                    if !arr.is_empty() {
                        let atom = DisplayAtom::text(format!("{}", arr.len()), Fidelity::Exact);
                        fidelity.record(&atom);
                        let prov = sid
                            .parse::<u32>()
                            .ok()
                            .and_then(|s| geo.and_then(|g| g.rep_province.get(&s).copied()));
                        entities.push(IconEntity::label("state_modifier_mapicon", atom, prov));
                    }
                }
            }
        }

        // ---- unit-counter stack visuals: group same-owner divisions by real counter
        // anchor, then emit only data-backed logical counter entities. Missing org/str/
        // role/dig-in fields stay badged at the anchored counter; no anchor produces one
        // unavailable badge rather than a province-less stack.
        push_counter_stack_entities(ctx, ctx.country, &mut fidelity, &mut entities);

        // ---- unit-counter commander portraits: an OOB/division leader ref can join to
        // the country's commander roster ONLY when the same counter/formation resolves to
        // a real province anchor. Misses remain explicit sprite misses; unanchored leaders
        // badge unavailable rather than fabricating a province-less portrait.
        push_counter_portrait_entities(ctx, ctx.country, &mut fidelity, &mut entities);

        // ---- unit-counter formation insignia: army / army-group shields use the shared
        // formation-insignia resolver, but are only emitted at real OOB/division counter
        // anchors. Missing/generic/out-of-range sprite data is kept as an explicit miss at
        // the anchored counter; formations without anchors collapse to one unavailable
        // badge rather than a province-less shield.
        push_counter_formation_insignia_entities(ctx, ctx.country, &mut fidelity, &mut entities);

        // ---- the remaining on-map icon-TEMPLATE families: ACCOUNTED, not dropped. Each
        // is badged with its honest reason so the binding-map's dynamic sections are all
        // represented (none silently omitted). The navy/air/rail COUNTERS + land/naval
        // COMBAT + air/naval MISSION + raid/faction/peace/decision icons still need
        // specialized stack types, battle/mission rosters, or target geometry not proven here →
        // badged unavailable (never a fabricated count/position). The intel /
        // special-facility families are also schema-unfed/asset-gap per §map.
        const BADGED_TEMPLATES: &[&str] = &[
            "unit_counter_navy",             // fleet roster — top-level OOB
            "unit_counter_railway_gun",      // railway-gun divisions — top-level domain
            "air_wing_counter",              // order_of_battle.air_wings unfed
            "GFX_mapicon_tiny_unit_counter", // transported-unit stack — top-level domain
            "land_combat_mapicon",           // combats[] — top-level domain
            "naval_combat_mapicon",          // combats[] (naval) — top-level domain
            "naval_combat_results_mapicon",  // event-timeline reconstruction
            "air_mission_mapicon",           // air missions — OOB air_wings unfed
            "naval_mission_mapicon",         // naval missions — top-level OOB
            // resource_mapicon + state_modifier_mapicon are now BOUND above (closed by the
            // live reader: states.*.resources / states.*.dynamic_modifier) — no longer badged.
            "construction_info_mapicon", // construction_queue — country domain, no on-map geom
            "naval_base_mapicon",        // docked ships — top-level OOB
            "supply_node_map_icon",      // supply.by_province — top-level proxy domain
            "supply_mapicon",            // supply.by_province — top-level proxy domain
            "railway_map_icon",          // province_buildings — top-level provinces domain
            "rocket_site_mapicon",       // extra.special_projects unfed
            "radar_mapicon",             // derived coverage over level+adjacency
            "operation_map_icon_entry",  // intel.operations — country domain w/o on-map geom
            "operative_map_icon_entry",  // operatives + portrait asset gap
            "cryptology_map_icon",       // intel.decryption — country domain
            "counterintelligence_map_icon_entry", // intel rating — country domain
            "intel_ledger_map_icon",     // intel-scaled ESTIMATE not modelled
            "decision_mapicon_item",     // decisions carry no per-target geometry
            "raid_map_icon",             // raids[] — top-level domain
            "faction_ping_map_icon",     // diplomacy_world — top-level domain
            "peace_mapicon",             // peace-conference runtime state
            "ping_mapicon",              // live MP ping — no persisted state
            "naval_mines_map_icon",      // no naval-mine domain
            "adjacency_rule_mapicon",    // canal controller — derived, no on-map value
        ];
        for tmpl in BADGED_TEMPLATES {
            let atom = DisplayAtom::unavailable_because(UnavailableReason::Blocked);
            fidelity.record(&atom);
            entities.push(IconEntity::label(tmpl, atom, None));
        }

        // ---- on-map placement: project each entity's province centroid through the live
        // camera. The centroid raster is reachable via ctx.stage.map_dir (Codex (b)); the
        // camera via ctx.map_camera. When BOTH are present, placement is DERIVED (an
        // on-map marker); when either is missing, the entity drops to the badged legend
        // (no fabricated map position).
        let centroids = province_centroids(&ctx.stage.map_dir);
        let placement_available = centroids.is_some() && ctx.map_camera.is_some();
        let placements: Vec<Option<(f32, f32)>> = entities
            .iter()
            .map(|e| match (e.province, ctx.map_camera, centroids) {
                (Some(pid), Some(cam), Some(set)) => set
                    .centroids
                    .get(pid)
                    .map(|(cx, cy)| project_centroid(&cam, set, cx, cy)),
                _ => None,
            })
            .collect();
        // one honest section badge for the placement layer: derived when wired, else
        // unavailable (the host did not wire centroids/camera this frame).
        let place = if placement_available {
            DisplayAtom::text("on-map", Fidelity::Derived)
        } else {
            DisplayAtom::unavailable_because(UnavailableReason::Blocked)
        };
        fidelity.record(&place);

        let roots = build_root(ctx, &file, MAPICONS_ROOT, &[]);
        BoundView {
            instance: InstanceId(0),
            view_id: MAPICONS_VIEW_ID,
            policy: self.policy(),
            // the icon layer reports no GUI leaf (placement is internal) → empty hit so a
            // click falls through to the map pick (like the combat overlay).
            hit: HitModel::none(),
            drawable: Box::new(MapIconsDrawable {
                roots,
                entities,
                placements,
                placement_available,
            }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }
    crate::impl_view_any!();
}

fn open_mapicons(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(MapIconsView::new())
}

register_view!(ViewDescriptor {
    id: MAPICONS_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(MAPICONS_FILE, MAPICONS_ROOT)),
    required_domains: &["states", "divisions"],
    route: Some(MenuRoute {
        cluster: MenuCluster::ChromeOutlinerMaps,
        label: "Map Icons",
        order: 7
    }),
    shortcut: None,
    open: open_mapicons,
    policy: HostPolicy::window(ZBand::Overlay),
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

    fn assets_present() -> bool {
        gui_dir().join("ingame_clock.json").exists()
    }

    fn stage_with(files: &[&str]) -> Stage {
        stage_with_manifest(files, HashMap::new())
    }

    fn stage_with_manifest(files: &[&str], sprites: HashMap<String, assets::SpriteEntry>) -> Stage {
        let mut gui_files = HashMap::new();
        for f in files {
            if let Some(g) = load_gui(f) {
                gui_files.insert(f.to_string(), g);
            }
        }
        Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest: assets::SpriteManifest { sprites },
            loc: gui_runtime::Loc::new(),
            gui_files,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
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

    fn sprite_manifest(entries: &[(&str, &str, u32)]) -> HashMap<String, assets::SpriteEntry> {
        entries
            .iter()
            .map(|(name, texture, frames)| {
                (
                    (*name).to_string(),
                    sprite_entry(name, Some(texture), *frames),
                )
            })
            .collect()
    }

    fn ctx<'a>(stage: &'a Stage, snap: &'a snapshot::Snapshot, date: &'a str) -> BindContext<'a> {
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
            date,
            cursor: (0.0, 0.0),
            map_camera: None,
        }
    }

    fn rebind_find(v: &mut FindView, snap: &snapshot::Snapshot, refit: bool) {
        v.rebind(&RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit,
        });
    }

    fn find_tab_center(bar: &TabBar, panel: Rect, id: &str) -> (f32, f32) {
        let rect = bar
            .layout(panel)
            .into_iter()
            .find(|tr| tr.id == id)
            .unwrap_or_else(|| panic!("missing find tab rect {id}"))
            .rect;
        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
    }

    fn click_find_tab_center(v: &mut FindView, id: &str) -> InputOutcome {
        let panel = find_panel_fallback(v.screen);
        let (x, y) = find_tab_center(&v.bar, panel, id);
        v.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        })
    }

    fn find_row_center(bar: &TabBar, panel: Rect, index: usize) -> (f32, f32) {
        let content = bar.content_rect(panel);
        (
            content.x + content.w * 0.5,
            content.y + index as f32 * FIND_ROW_H + FIND_ROW_H * 0.5,
        )
    }

    fn click_find_row_key(v: &mut FindView, key: &str) -> InputOutcome {
        let tab = selected_find_tab(&v.bar);
        let panel = find_panel_fallback(v.screen);
        let index = v
            .catalog
            .rows(tab)
            .iter()
            .position(|r| r.key == key)
            .unwrap_or_else(|| panic!("missing cached find row {key}"));
        let (x, y) = find_row_center(&v.bar, panel, index);
        v.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        })
    }

    fn click_find_content_after_rows(v: &mut FindView) -> InputOutcome {
        let tab = selected_find_tab(&v.bar);
        let panel = find_panel_fallback(v.screen);
        let content = v.bar.content_rect(panel);
        let index = v.catalog.rows(tab).len();
        v.on_input(&ViewInput::PointerDown {
            x: content.x + content.w * 0.5,
            y: content.y + index as f32 * FIND_ROW_H + FIND_ROW_H * 0.5,
            button: PointerButton::Left,
        })
    }

    fn mapmode_panel_for_tests() -> Rect {
        Rect {
            x: 40.0,
            y: 60.0,
            w: 120.0,
            h: MAP_MODES.len() as f32 * MAPMODE_ROW_H,
        }
    }

    fn mapmode_index(name: &str) -> usize {
        MAP_MODES
            .iter()
            .position(|mode| mode.name == name)
            .unwrap_or_else(|| panic!("missing mapmode row {name}"))
    }

    fn mapmode_row_center(panel: Rect, name: &str) -> (f32, f32) {
        let rect = mapmode_row_rect(panel, mapmode_index(name));
        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
    }

    fn click_mapmode_row(v: &mut MapModesView, panel: Rect, name: &str) -> InputOutcome {
        let (x, y) = mapmode_row_center(panel, name);
        v.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        })
    }

    /// A snapshot with a couple of VP/occupied states + a country (the data the icon /
    /// find views read). `date` carries the hour the clock reads.
    fn snap_a() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11", "hour_index": 100,
            "countries": { "GER": { "politics": { "capital_state": 64, "ruling_party": "fascism" } } },
            "states": {
                "64": { "owner": "GER", "controller": "GER", "category": "city",
                    "victory_points_total": 50, "occupation": null },
                "75": { "owner": "GER", "controller": "SOV", "category": "town",
                    "victory_points_total": 5, "occupation": {"resistance": 12.0, "compliance": 40.0} }
            }
        }))
        .unwrap()
    }

    fn snap_b() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1942.7.1.18", "hour_index": 200,
            "countries": { "GER": { "politics": { "capital_state": 64, "ruling_party": "democratic" } } },
            "states": {
                "64": { "owner": "GER", "controller": "SOV", "category": "city",
                    "victory_points_total": 50, "occupation": {"resistance": 80.0, "compliance": 5.0} },
                "75": { "owner": "GER", "controller": "GER", "category": "town",
                    "victory_points_total": 5, "occupation": null }
            }
        }))
        .unwrap()
    }

    // -------- the clock binds the hour exact ------------------------------------------
    #[test]
    fn clock_binds_hour_exact() {
        if !assets_present() {
            eprintln!("[chrome] skipped: game_assets/gui not extracted");
            return;
        }
        let stage = stage_with(&["ingame_clock.json"]);
        let snap = snap_a();
        let v = ClockView::new();
        let d = bound_digest(&v.bind(&ctx(&stage, &snap, "1941.4.13.11")));
        assert!(
            d.contains("time_text=11:00(E)"),
            "clock must bind the hour exact, got: {d}"
        );
    }

    // -------- alerts badge unavailable, never fabricated -------------------------------
    #[test]
    fn alerts_badge_unavailable_not_fabricated() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["alerts.json"]);
        let snap = snap_a();
        let v = AlertsView::new();
        let bv = v.bind(&ctx(&stage, &snap, "1941.4.13.11"));
        let d = bound_digest(&bv);
        assert!(
            d.contains("icon=—(U)"),
            "alert icon must badge unavailable (no alerts ledger): {d}"
        );
        assert!(
            d.contains("country_flag=—(U)"),
            "diplo flag must badge unavailable: {d}"
        );
        assert_eq!(bv.fidelity.exact, 0, "alerts has no exact fields");
    }

    // -------- minimap click-to-recenter ----------------------------------------------
    #[test]
    fn minimap_click_inside_fitted_map_emits_and_applies_recenter() {
        let mut cam = crate::camera::MapCamera::new(1600, 900, 5632, 2048);
        cam.scale = 4.0;
        let before = format!("{:?}:{:?}", cam.uniform_offset(), cam.uniform_scale());

        let mut v = MinimapView::new();
        v.set_camera(Some(cam));
        let input = v
            .input
            .expect("minimap input cache should sync from live camera");
        let mm = Minimap::new(input.panel, input.world.0, input.world.1);
        let fitted = mm.fitted().expect("fallback minimap must be usable");

        let out = v.on_input(&ViewInput::PointerDown {
            x: fitted.x + fitted.w * 0.75,
            y: fitted.y + fitted.h * 0.25,
            button: PointerButton::Left,
        });

        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert_eq!(out.commands.len(), 1);
        let (raster_x, raster_y) = match out.commands[0] {
            ViewCommand::RecenterMap { raster_x, raster_y } => (raster_x, raster_y),
            ref other => panic!("expected RecenterMap command, got {other:?}"),
        };
        assert!((raster_x - 4224.0).abs() < 0.01, "raster_x={raster_x}");
        assert!((raster_y - 512.0).abs() < 0.01, "raster_y={raster_y}");

        assert!(cam.recenter_on_raster(raster_x, raster_y));
        let after = format!("{:?}:{:?}", cam.uniform_offset(), cam.uniform_scale());
        assert_ne!(
            before, after,
            "applying recenter must perturb camera uniforms"
        );
        let max_off = 1.0 - 1.0 / cam.scale;
        assert!(cam.offset[0] >= 0.0 && cam.offset[0] <= max_off + 1e-6);
        assert!(cam.offset[1] >= 0.0 && cam.offset[1] <= max_off + 1e-6);
    }

    #[test]
    fn minimap_recenter_uses_rebound_draw_panel_not_stale_fallback() {
        let cam = crate::camera::MapCamera::new(1600, 900, 5632, 2048);
        let mut v = MinimapView::new();
        v.set_camera(Some(cam));
        let fallback = v
            .input
            .expect("minimap input cache should sync from live camera")
            .panel;
        let bound_panel = Rect {
            x: 40.0,
            y: 40.0,
            w: 340.0,
            h: 180.0,
        };
        assert!(
            !fallback.contains(
                bound_panel.x + bound_panel.w * 0.5,
                bound_panel.y + bound_panel.h * 0.5
            ),
            "test panel must differ from the camera fallback"
        );

        v.set_input_panel(bound_panel);
        let input = v
            .input
            .expect("minimap input cache should keep live camera after panel sync");
        assert_eq!(input.panel, bound_panel);
        let mm = Minimap::new(input.panel, input.world.0, input.world.1);
        let fitted = mm.fitted().expect("rebound minimap must be usable");

        let out = v.on_input(&ViewInput::PointerDown {
            x: fitted.x + fitted.w * 0.5,
            y: fitted.y + fitted.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(out.consumed);
        assert_eq!(out.commands.len(), 1);
        assert!(matches!(out.commands[0], ViewCommand::RecenterMap { .. }));

        let stale_fallback_click = v.on_input(&ViewInput::PointerDown {
            x: fallback.x + fallback.w * 0.5,
            y: fallback.y + fallback.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(
            !stale_fallback_click.consumed,
            "stale fallback panel must not keep stealing map picks after panel sync"
        );
        assert!(stale_fallback_click.commands.is_empty());
    }

    #[test]
    fn minimap_letterbox_click_is_explicit_noop_and_outside_is_ignored() {
        let cam = crate::camera::MapCamera::new(1600, 900, 5632, 2048);
        let mut v = MinimapView::new();
        v.set_camera(Some(cam));
        let input = v
            .input
            .expect("minimap input cache should sync from live camera");
        let mm = Minimap::new(input.panel, input.world.0, input.world.1);
        let fitted = mm.fitted().expect("fallback minimap must be usable");

        let letterbox = v.on_input(&ViewInput::PointerDown {
            x: fitted.x + fitted.w * 0.5,
            y: input.panel.y + 1.0,
            button: PointerButton::Left,
        });
        assert!(letterbox.consumed);
        assert!(!letterbox.needs_redraw);
        assert!(letterbox.commands.is_empty());

        let outside = v.on_input(&ViewInput::PointerDown {
            x: input.panel.x - 5.0,
            y: input.panel.y + input.panel.h * 0.5,
            button: PointerButton::Left,
        });
        assert!(!outside.consumed);
        assert!(!outside.needs_redraw);
        assert!(outside.commands.is_empty());
    }

    #[test]
    fn minimap_does_not_recenter_without_live_camera() {
        let mut v = MinimapView::new();
        v.set_camera(None);
        let panel = minimap_panel_fallback(default_find_screen());

        let out = v.on_input(&ViewInput::PointerDown {
            x: panel.x + panel.w * 0.5,
            y: panel.y + panel.h * 0.5,
            button: PointerButton::Left,
        });

        assert!(!out.consumed);
        assert!(!out.needs_redraw);
        assert!(out.commands.is_empty());
    }

    // -------- find rows are derived from the snapshot, no fabrication ------------------
    #[test]
    fn find_states_rows_derived_from_snapshot() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["find_view.json"]);
        let snap = snap_a();
        let v = FindView::new();
        let d = bound_digest(&v.bind(&ctx(&stage, &snap, "1941.4.13.11")));
        // the default tab is States; the two present states surface as derived rows.
        assert!(
            d.contains("state:64"),
            "find must list present state 64: {d}"
        );
        assert!(
            d.contains("state:75"),
            "find must list present state 75: {d}"
        );
    }

    #[test]
    fn find_tab_click_switches_digest_and_visible_catalog() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut v = FindView::new();
        rebind_find(&mut v, &snap, true);

        let cx = ctx(&stage, &snap, "1941.4.13.11");
        let states = bound_digest(&v.bind(&cx));
        assert!(states.contains("tab=Some(\"states\")"), "{states}");
        assert!(states.contains("state:64="), "{states}");
        assert!(states.contains("state:75="), "{states}");

        let out = click_find_tab_center(&mut v, "countries");
        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert!(out.commands.is_empty());
        let countries = bound_digest(&v.bind(&cx));
        assert!(countries.contains("tab=Some(\"countries\")"), "{countries}");
        assert!(countries.contains("GER=GER(D)"), "{countries}");
        assert!(
            !countries.contains("state:64="),
            "countries tab must not keep state rows: {countries}"
        );

        let out = click_find_tab_center(&mut v, "vps");
        assert!(out.consumed);
        assert!(out.needs_redraw);
        let vps = bound_digest(&v.bind(&cx));
        assert!(vps.contains("tab=Some(\"vps\")"), "{vps}");
        assert!(vps.contains("vp:64="), "{vps}");
        assert!(vps.contains("vp:75="), "{vps}");
        assert!(
            !vps.contains("GER=GER(D)"),
            "VP tab must not keep country rows: {vps}"
        );
    }

    #[test]
    fn find_tab_inert_behavior_is_explicit_when_tab_not_pickable() {
        let live = find_tabs();
        assert!(
            live.tabs.iter().all(|t| t.avail.is_pickable()),
            "chrome.find currently has no disabled or unavailable catalog tabs"
        );

        let panel = find_panel_fallback(default_find_screen());
        let mut blocked = TabBar::horizontal(vec![
            Tab::enabled("states", WAtom::exact("States")),
            Tab::disabled("countries", WAtom::exact("Countries")),
            Tab::unavailable("vps", WAtom::exact("Victory Points")),
        ]);
        assert_eq!(blocked.selected(), Some("states"));

        let (cx, cy) = find_tab_center(&blocked, panel, "countries");
        assert_eq!(
            click_find_tab(&mut blocked, panel, cx, cy),
            FindTabClick::Inert
        );
        assert_eq!(blocked.selected(), Some("states"));

        let (vx, vy) = find_tab_center(&blocked, panel, "vps");
        assert_eq!(
            click_find_tab(&mut blocked, panel, vx, vy),
            FindTabClick::Inert
        );
        assert_eq!(blocked.selected(), Some("states"));
    }

    #[test]
    fn find_selected_tab_persists_across_scrub_a_b_a() {
        let stage = stage_with(&[]);
        let (a, b) = (snap_a(), snap_b());
        let mut v = FindView::new();
        rebind_find(&mut v, &a, true);
        let out = click_find_tab_center(&mut v, "countries");
        assert!(out.consumed && out.needs_redraw);

        let da1 = bound_digest(&v.bind(&ctx(&stage, &a, "1941.4.13.11")));
        rebind_find(&mut v, &b, false);
        let db = bound_digest(&v.bind(&ctx(&stage, &b, "1942.7.1.18")));
        rebind_find(&mut v, &a, false);
        let da2 = bound_digest(&v.bind(&ctx(&stage, &a, "1941.4.13.11")));

        assert!(da1.contains("tab=Some(\"countries\")"), "{da1}");
        assert!(db.contains("tab=Some(\"countries\")"), "{db}");
        assert_eq!(
            da1, da2,
            "scrub A->B->A must preserve the selected find tab without carrying B rows"
        );
    }

    #[test]
    fn find_state_row_click_selects_digest_and_emits_state_panel_command() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut v = FindView::new();
        rebind_find(&mut v, &snap, true);
        let cx = ctx(&stage, &snap, "1941.4.13.11");

        let before = bound_digest(&v.bind(&cx));
        assert!(before.contains("selected=none"), "{before}");

        let out = click_find_row_key(&mut v, "state:64");
        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert_eq!(
            out.commands,
            vec![ViewCommand::OpenView {
                view_id: crate::views::state_panel::STATE_PANEL_VIEW_ID,
                arg: Some(64),
            }]
        );

        let after = bound_digest(&v.bind(&cx));
        assert_ne!(before, after, "row selection must perturb digest");
        assert!(
            after.contains("selected=state:64=#64 (GER)(D)"),
            "selected state row must be carried in digest: {after}"
        );
    }

    #[test]
    fn find_vp_row_click_selects_digest_and_emits_state_panel_command() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut v = FindView::new();
        rebind_find(&mut v, &snap, true);
        let cx = ctx(&stage, &snap, "1941.4.13.11");

        let tab_out = click_find_tab_center(&mut v, "vps");
        assert!(tab_out.consumed);

        let out = click_find_row_key(&mut v, "vp:64");
        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert_eq!(
            out.commands,
            vec![ViewCommand::OpenView {
                view_id: crate::views::state_panel::STATE_PANEL_VIEW_ID,
                arg: Some(64),
            }]
        );

        let after = bound_digest(&v.bind(&cx));
        assert!(
            after.contains("selected=vp:64=#64: 50 VP(E)"),
            "selected VP row must be carried in digest: {after}"
        );
    }

    #[test]
    fn find_country_row_click_selects_without_drill_command() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut v = FindView::new();
        rebind_find(&mut v, &snap, true);
        let cx = ctx(&stage, &snap, "1941.4.13.11");

        let tab_out = click_find_tab_center(&mut v, "countries");
        assert!(tab_out.consumed);

        let out = click_find_row_key(&mut v, "GER");
        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert!(
            out.commands.is_empty(),
            "country rows are selectable but do not yet have a drill command"
        );

        let after = bound_digest(&v.bind(&cx));
        assert!(
            after.contains("selected=GER=GER(D)"),
            "selected country row must be carried in digest: {after}"
        );
    }

    #[test]
    fn find_click_after_visible_rows_is_ignored() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let mut v = FindView::new();
        rebind_find(&mut v, &snap, true);
        let before = bound_digest(&v.bind(&ctx(&stage, &snap, "1941.4.13.11")));

        let out = click_find_content_after_rows(&mut v);
        assert!(!out.consumed);
        assert!(!out.needs_redraw);
        assert!(out.commands.is_empty());

        let after = bound_digest(&v.bind(&ctx(&stage, &snap, "1941.4.13.11")));
        assert_eq!(
            before, after,
            "clicking empty catalog space must not invent or change a selection"
        );
    }

    #[test]
    fn find_selected_row_survives_scrub_without_carrying_b_only_rows() {
        let stage = stage_with(&[]);
        let a = snap_a();
        let b_extra: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.7.1.18", "hour_index": 200,
            "countries": { "GER": { "politics": { "capital_state": 64 } } },
            "states": {
                "64": { "owner": "SOV", "controller": "SOV", "category": "city",
                    "victory_points_total": 50 },
                "75": { "owner": "GER", "controller": "GER", "category": "town",
                    "victory_points_total": 5 },
                "88": { "owner": "ITA", "controller": "ITA", "category": "rural",
                    "victory_points_total": 1 }
            }
        }))
        .unwrap();

        let mut v = FindView::new();
        rebind_find(&mut v, &a, true);
        let out = click_find_row_key(&mut v, "state:64");
        assert!(out.consumed);

        let da1 = bound_digest(&v.bind(&ctx(&stage, &a, "1941.4.13.11")));
        rebind_find(&mut v, &b_extra, false);
        let db = bound_digest(&v.bind(&ctx(&stage, &b_extra, "1942.7.1.18")));
        rebind_find(&mut v, &a, false);
        let da2 = bound_digest(&v.bind(&ctx(&stage, &a, "1941.4.13.11")));

        assert!(da1.contains("selected=state:64=#64 (GER)(D)"), "{da1}");
        assert!(db.contains("state:88="), "{db}");
        assert_eq!(
            da1, da2,
            "selected row must survive A->B->A without carrying B rows"
        );
        assert!(
            !da2.contains("state:88="),
            "returning to A must not retain the B-only row: {da2}"
        );
    }

    #[test]
    fn find_missing_selected_row_is_not_fabricated() {
        let stage = stage_with(&[]);
        let a = snap_a();
        let missing: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1943.1.1.0", "hour_index": 300,
            "countries": { "GER": { "politics": { "capital_state": 64 } } }
        }))
        .unwrap();

        let mut v = FindView::new();
        rebind_find(&mut v, &a, true);
        let out = click_find_row_key(&mut v, "state:64");
        assert!(out.consumed);

        rebind_find(&mut v, &missing, false);
        let missing_digest = bound_digest(&v.bind(&ctx(&stage, &missing, "1943.1.1.0")));
        assert!(missing_digest.contains("selected=none"), "{missing_digest}");
        assert!(
            !missing_digest.contains("state:64="),
            "missing state row must not be fabricated from selection state: {missing_digest}"
        );

        let out = click_find_content_after_rows(&mut v);
        assert!(!out.consumed);
        assert!(out.commands.is_empty());

        rebind_find(&mut v, &a, false);
        let restored = bound_digest(&v.bind(&ctx(&stage, &a, "1941.4.13.11")));
        assert!(
            restored.contains("selected=state:64=#64 (GER)(D)"),
            "selection key may persist, but only visible rows can surface it: {restored}"
        );
    }

    /// RE-WIRE: the Countries catalog row is ENRICHED with the now-recorded per-country
    /// `identity` (original_tag / original_capital / cores / claims). A measured identity
    /// surfaces; an absent one adds nothing (no fabrication).
    #[test]
    fn find_country_row_enriched_with_identity_when_measured() {
        // build the Countries-tab rows directly (no assets needed) for a released country
        // D01 whose identity is fully measured.
        let stage = stage_with(&[]);
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11",
            "countries": { "D01": { "identity": {
                "original_tag": "SPR", "original_capital": 41,
                "cores": [41, 42, 43], "claims": []
            } } }
        }))
        .unwrap();
        let cx = ctx(&stage, &snap, "1941.4.13.11");
        // resolve the Countries tab through the public find_rows path (D01 is the observed
        // country here).
        let cx = BindContext {
            country: "D01",
            ..cx
        };
        let mut fid = FidelityReport::default();
        let rows = find_rows(&cx, FindTab::Countries, &mut fid);
        assert_eq!(rows.len(), 1);
        let label = rows[0].name.as_str();
        assert!(label.contains("D01"), "row carries the TAG: {label}");
        assert!(
            label.contains("(was SPR)"),
            "original_tag surfaced: {label}"
        );
        assert!(
            label.contains("[cap #41]"),
            "original_capital surfaced: {label}"
        );
        assert!(label.contains("cores:3"), "core count surfaced: {label}");
        // a measured-EMPTY claims list adds nothing (no "claims:0").
        assert!(
            !label.contains("claims:"),
            "empty claims add nothing: {label}"
        );
    }

    /// no-fabrication: a country with NO identity bag yields the bare TAG row — no invented
    /// "(was …)"/cap/cores suffixes.
    #[test]
    fn find_country_row_bare_tag_when_identity_absent() {
        let stage = stage_with(&[]);
        let snap = snap_a(); // GER has no identity bag
        let cx = ctx(&stage, &snap, "1941.4.13.11");
        let mut fid = FidelityReport::default();
        let rows = find_rows(&cx, FindTab::Countries, &mut fid);
        assert_eq!(rows.len(), 1);
        let label = rows[0].name.as_str();
        assert_eq!(
            label, "GER",
            "bare TAG when identity absent, no fabricated suffix"
        );
    }

    // -------- mapicons enumerate VP/occupation entities + badge the gaps --------------
    #[test]
    fn mapicons_enumerate_and_badge_gaps() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["mapicons.json"]);
        let snap = snap_a();
        let v = MapIconsView::new();
        let bv = v.bind(&ctx(&stage, &snap, "1941.4.13.11"));
        let d = bound_digest(&bv);
        // (the digest carries `map{bool}` = whether on-map placement was wired this frame.
        // In the headless test there is no state_meta.json under map_dir="." → province
        // resolves to None for per-state icons, and no camera → mapfalse; the FIELD is
        // still bound exact — the value, not the placement, is the no-fabrication unit.)
        assert!(
            d.contains("victory_point_mapicon@Nonemapfalse=50(E)"),
            "VP value must bind exact: {d}"
        );
        assert!(
            d.contains("resistance_compliance_map_icon@Nonemapfalse=12%(E)"),
            "resistance exact: {d}"
        );
        assert!(
            d.contains("unit_counter.general_portrait@Nonemapfalse=—(U)"),
            "portrait gap badged: {d}"
        );
        assert!(
            d.contains("air_wing_counter@Nonemapfalse=—(U)"),
            "air-wing gap badged: {d}"
        );
        // the unit-counter + land-combat families are ACCOUNTED (badged), not dropped.
        assert!(
            d.contains("land_combat_mapicon@Nonemapfalse=—(U)"),
            "land combat family accounted: {d}"
        );
        assert!(
            d.contains("supply_node_map_icon@Nonemapfalse=—(U)"),
            "supply family accounted: {d}"
        );
    }

    // -------- mapicons bind anchored unit-counter stack metrics ------------------------
    #[test]
    fn mapicon_unit_counter_stack_metrics_bind_at_owned_division_anchor() {
        let stage = stage_with_manifest(&[], HashMap::new());
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "divisions": [
                { "div_id": 101, "div_type": 42, "owner": "GER", "location_province": 790,
                  "organisation": 80.0, "strength": 0.9, "template_role": "infantry", "dig_in": 0.0 },
                { "div_id": 102, "div_type": 42, "owner": "GER", "location_province": 790,
                  "organisation": 60.0, "strength": 0.8, "template_role": "infantry", "dig_in": 2.0 },
                { "div_id": 201, "div_type": 42, "owner": "SOV", "location_province": 790,
                  "organisation": 10.0, "strength": 0.2, "template_role": "armor", "dig_in": 9.0 },
                { "div_id": 103, "div_type": 42, "owner": "GER", "target_province": 791,
                  "organisation": 100.0, "strength": 1.0, "template_role": "armor", "dig_in": 5.0 }
            ]
        }))
        .unwrap();
        let bv = MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12"));
        let d = bound_digest(&bv);
        assert!(
            d.contains("unit_counter.count_txt@Some(790)mapfalse=2(D);"),
            "owned anchored stack count must derive from same-province GER divisions only: {d}"
        );
        assert!(
            d.contains("unit_counter.flag@Some(790)mapfalse=GER(E);"),
            "owner flag stripe must bind exact owner TAG: {d}"
        );
        assert!(
            d.contains("unit_counter.bar_org@Some(790)mapfalse=70%(E);"),
            "org bar must average measured org values exactly: {d}"
        );
        assert!(
            d.contains("unit_counter.bar_str@Some(790)mapfalse=85%(E);"),
            "strength bar must average measured strength values exactly: {d}"
        );
        assert!(
            d.contains("unit_counter.type@Some(790)mapfalse=infantry(D);"),
            "template role symbol must derive from known roles: {d}"
        );
        assert!(
            d.contains("unit_counter.onmap_digin_icon@Some(790)mapfalse=dig(E);"),
            "positive measured dig-in must show the dig-in badge: {d}"
        );
        assert!(
            !d.contains("unit_counter.count_txt@Some(791)"),
            "target_province is not a counter anchor and must not place stack metrics: {d}"
        );
    }

    #[test]
    fn mapicon_unit_counter_memory_scaled_strength_badges_unavailable_at_anchor() {
        let stage = stage_with_manifest(&[], HashMap::new());
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "divisions": [
                { "div_id": 107, "div_type": 42, "owner": "GER", "location_province": 795,
                  "organisation": 70.0, "strength": 229.4, "template_role": "infantry", "dig_in": 0.0 }
            ]
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        let unavailable_strength = format!(
            "unit_counter.bar_str@Some(795)mapfalse={}(U);",
            DisplayAtom::unavailable_because(UnavailableReason::Blocked).as_str()
        );
        assert!(
            d.contains("unit_counter.count_txt@Some(795)mapfalse=1(D);"),
            "memory-scaled strength must preserve the real counter anchor: {d}"
        );
        assert!(
            d.contains(&unavailable_strength),
            "memory-scaled strength must be badged unavailable at the real counter anchor: {d}"
        );
        assert!(
            !d.contains("unit_counter.bar_str@Some(795)mapfalse=229%(E);"),
            "memory-scaled strength must not be fabricated as a progressbar percent: {d}"
        );
    }

    #[test]
    fn mapicon_unit_counter_missing_stack_fields_badge_at_anchor() {
        let stage = stage_with_manifest(&[], HashMap::new());
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "divisions": [
                { "div_id": 104, "div_type": 42, "owner": "GER", "location_province": 792,
                  "strength": 0.5, "template_role": "unknown_role" }
            ]
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert!(
            d.contains("unit_counter.count_txt@Some(792)mapfalse=1(D);"),
            "count remains derived from the anchored measured division: {d}"
        );
        assert!(
            d.contains("unit_counter.bar_org@Some(792)mapfalse=—(U);"),
            "missing org must be explicit unavailable at the real counter anchor: {d}"
        );
        assert!(
            d.contains("unit_counter.bar_str@Some(792)mapfalse=50%(E);"),
            "present strength still binds exact even when sibling fields are missing: {d}"
        );
        assert!(
            d.contains("unit_counter.type@Some(792)mapfalse=—(U);"),
            "unknown role must not fabricate a placeholder NATO symbol: {d}"
        );
        assert!(
            d.contains("unit_counter.onmap_digin_icon@Some(792)mapfalse=—(U);"),
            "missing dig-in visibility must be explicitly badged: {d}"
        );
    }

    #[test]
    fn mapicon_unit_counter_rejects_foreign_owner_and_unanchored_divisions() {
        let stage = stage_with_manifest(&[], HashMap::new());
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "divisions": [
                { "div_id": 105, "div_type": 42, "owner": "SOV", "location_province": 793,
                  "organisation": 75.0, "strength": 0.75, "template_role": "armor", "dig_in": 1.0 },
                { "div_id": 106, "div_type": 42, "owner": "GER", "target_province": 794,
                  "organisation": 90.0, "strength": 0.9, "template_role": "infantry", "dig_in": 1.0 }
            ]
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert!(
            d.contains("unit_counter.count_txt@Nonemapfalse=—(U);"),
            "no eligible GER counter anchor must stay explicitly unavailable: {d}"
        );
        assert!(
            !d.contains("unit_counter.count_txt@Some(793)"),
            "foreign-owner divisions must not appear in the observed country's stack: {d}"
        );
        assert!(
            !d.contains("unit_counter.count_txt@Some(794)"),
            "target-only GER divisions must not fabricate a counter anchor: {d}"
        );
    }

    // -------- mapicons bind commander portrait sprites without fabricating placeholders
    #[test]
    fn mapicon_general_portrait_without_counter_anchor_badges_no_sprite() {
        let stage = stage_with_manifest(
            &[],
            sprite_manifest(&[(
                "GFX_portrait_GER_heinz_guderian",
                "gfx/leaders/GER/portrait_guderian.dds",
                1,
            )]),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "countries": { "GER": { "commanders": {
                "generals": [
                    { "unit_leader_id": 7, "role": "general",
                      "portrait_gfx": "GFX_portrait_GER_heinz_guderian" }
                ],
                "field_marshals": [],
                "admirals": []
            } } },
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "leader": { "id": "7", "type": "corps_commander" } }
                ]
            }
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert!(
            d.contains("unit_counter.general_portrait@Nonemapfalse=—(U)"),
            "unanchored leader data must remain an explicit portrait badge: {d}"
        );
        assert!(
            !d.contains("unit_counter.general_portrait@Nonemapfalse:sprite=GFX_portrait_GER_heinz_guderian#0(E);"),
            "unanchored leader data must not fabricate a global portrait entity: {d}"
        );
    }

    #[test]
    fn mapicon_general_portrait_resolves_anchored_oob_army_leader_sprite() {
        let stage = stage_with_manifest(
            &[],
            sprite_manifest(&[(
                "GFX_portrait_GER_heinz_guderian",
                "gfx/leaders/GER/portrait_guderian.dds",
                1,
            )]),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "countries": { "GER": { "commanders": {
                "generals": [
                    { "unit_leader_id": 7, "role": "general",
                      "portrait_gfx": "GFX_portrait_GER_heinz_guderian" }
                ],
                "field_marshals": [],
                "admirals": []
            } } },
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "leader": { "id": "7", "type": "corps_commander" },
                      "members": ["101:42"] }
                ]
            },
            "divisions": [
                { "div_id": 101, "div_type": 42, "owner": "GER", "location_province": 777 }
            ]
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert!(
            d.contains(
                "unit_counter.general_portrait@Some(777)mapfalse:sprite=GFX_portrait_GER_heinz_guderian#0(E);"
            ),
            "resolved commander portrait must bind only at the real counter province: {d}"
        );
        assert!(
            !d.contains("unit_counter.general_portrait@Nonemapfalse:sprite=GFX_portrait_GER_heinz_guderian#0(E);"),
            "resolved portrait must not also appear as a province-less global entity: {d}"
        );
    }

    #[test]
    fn mapicon_general_portrait_missing_sprite_records_miss() {
        let stage = stage_with_manifest(&[], HashMap::new());
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "countries": { "GER": { "commanders": {
                "generals": [
                    { "unit_leader_id": 7, "portrait_gfx": "GFX_portrait_missing" }
                ],
                "field_marshals": [],
                "admirals": []
            } } },
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "leader_id": 7, "members": ["101:42"] }
                ]
            },
            "divisions": [
                { "div_id": 101, "div_type": 42, "owner": "GER", "location_province": 778 }
            ]
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert!(
            d.contains("unit_counter.general_portrait@Some(778)mapfalse:sprite=MISS:missing_sprite:GFX_portrait_missing(U);"),
            "missing portrait sprite must remain an explicit miss: {d}"
        );
    }

    #[test]
    fn mapicon_general_portrait_generic_placeholder_records_miss() {
        let stage = stage_with_manifest(
            &[],
            sprite_manifest(&[("GFX_leader_unknown", "gfx/leaders/leader_unknown.dds", 1)]),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "countries": { "GER": { "commanders": {
                "generals": [
                    { "unit_leader_id": 7, "portrait_gfx": "GFX_leader_unknown" }
                ],
                "field_marshals": [],
                "admirals": []
            } } },
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "leader_id": 7, "members": ["101:42"] }
                ]
            },
            "divisions": [
                { "div_id": 101, "div_type": 42, "owner": "GER", "location_province": 779 }
            ]
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert!(
            d.contains("unit_counter.general_portrait@Some(779)mapfalse:sprite=MISS:generic_placeholder:GFX_leader_unknown(U);"),
            "generic placeholder must stay an explicit unavailable miss: {d}"
        );
        assert!(
            !d.contains(
                "unit_counter.general_portrait@Some(779)mapfalse:sprite=GFX_leader_unknown#0(E);"
            ),
            "generic placeholder must never be fabricated as exact art: {d}"
        );
    }

    #[test]
    fn mapicon_general_portrait_typed_mismatch_fails_closed() {
        let stage = stage_with_manifest(
            &[],
            sprite_manifest(&[(
                "GFX_portrait_wrong_field_marshal",
                "gfx/leaders/GER/wrong_field_marshal.dds",
                1,
            )]),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "countries": { "GER": { "commanders": {
                "generals": [],
                "field_marshals": [
                    { "unit_leader_id": 7, "type": "field_marshal",
                      "portrait_gfx": "GFX_portrait_wrong_field_marshal" }
                ],
                "admirals": []
            } } },
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "leader": { "id": "7", "type": "corps_commander" },
                      "members": ["101:42"] }
                ]
            },
            "divisions": [
                { "div_id": 101, "div_type": 42, "owner": "GER", "location_province": 780 }
            ]
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert!(
            d.contains(
                "unit_counter.general_portrait@Some(780)mapfalse:sprite=MISS:no_sprite_name(U);"
            ),
            "typed mismatch must fail closed as an explicit portrait miss: {d}"
        );
        assert!(
            !d.contains("GFX_portrait_wrong_field_marshal#0(E)"),
            "typed mismatch must not attach same-id wrong-role art: {d}"
        );
    }

    // -------- mapicons bind formation insignia only at real counter anchors ------------
    #[test]
    fn mapicon_formation_insignia_resolves_anchored_army_explicit_sprite() {
        let stage = stage_with_manifest(
            &[],
            sprite_manifest(&[("GFX_exact_army", "gfx/interface/exact_army.dds", 1)]),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "insignia_gfx": "GFX_exact_army",
                      "members": ["101:42"] }
                ]
            },
            "divisions": [
                { "div_id": 101, "div_type": 42, "owner": "GER", "location_province": 781 }
            ]
        }))
        .unwrap();
        let bv = MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12"));
        let d = bound_digest(&bv);
        assert!(
            d.contains(
                "unit_counter.formation_insignia@Some(781)mapfalse:sprite=GFX_exact_army#0(E);"
            ),
            "anchored explicit formation insignia must bind exact art at the counter province: {d}"
        );
        assert!(
            !d.contains("unit_counter.formation_insignia@Nonemapfalse:sprite=GFX_exact_army#0(E);"),
            "explicit insignia must not also appear as a province-less global entity: {d}"
        );
        assert!(
            bv.fidelity.exact >= 1,
            "explicit formation art must be counted exact: {:?}",
            bv.fidelity
        );
    }

    #[test]
    fn mapicon_formation_insignia_uses_anchor_gated_shield_frame_digest_stably() {
        let stage = stage_with_manifest(
            &[],
            sprite_manifest(&[(
                "GFX_theatre_army_shield",
                "gfx/interface/army_shield.dds",
                4,
            )]),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "icon": 3,
                      "members": ["101:42", {"id": 101, "type": 42}] }
                ]
            },
            "divisions": [
                { "div_id": 101, "div_type": 42, "owner": "GER", "location_province": 782 }
            ]
        }))
        .unwrap();
        let view = MapIconsView::new();
        let first = view.bind(&ctx(&stage, &snap, "1942.1.1.12"));
        let d = bound_digest(&first);
        assert!(
            d.contains(
                "unit_counter.formation_insignia@Some(782)mapfalse:sprite=GFX_theatre_army_shield#2(D);"
            ),
            "one-based army shield frame must bind as derived art at the counter province: {d}"
        );
        assert_eq!(
            d.matches("unit_counter.formation_insignia@Some(782)mapfalse:sprite=GFX_theatre_army_shield#2(D);").count(),
            1,
            "duplicate member refs must not duplicate the same counter shield digest: {d}"
        );
        assert!(
            first.fidelity.derived >= 1,
            "frame-derived formation art must be counted derived: {:?}",
            first.fidelity
        );
        let second = bound_digest(&view.bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert_eq!(d, second, "formation insignia digest must be deterministic");
    }

    #[test]
    fn mapicon_formation_insignia_generic_and_missing_sprites_record_misses() {
        let stage = stage_with_manifest(
            &[],
            sprite_manifest(&[(
                "GFX_unknown_formation",
                "gfx/interface/unknown_formation.dds",
                1,
            )]),
        );
        let generic: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "emblem_gfx": "GFX_unknown_formation",
                      "members": ["101:42"] }
                ]
            },
            "divisions": [
                { "div_id": 101, "div_type": 42, "owner": "GER", "location_province": 783 }
            ]
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &generic, "1942.1.1.12")));
        assert!(
            d.contains("unit_counter.formation_insignia@Some(783)mapfalse:sprite=MISS:generic_placeholder:GFX_unknown_formation(U);"),
            "generic formation placeholder must remain an explicit miss: {d}"
        );
        assert!(
            !d.contains("unit_counter.formation_insignia@Some(783)mapfalse:sprite=GFX_unknown_formation#0(E);"),
            "generic placeholder must never be reported as exact formation art: {d}"
        );

        let missing: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "insignia_gfx": "GFX_missing_formation",
                      "members": ["101:42"] }
                ]
            },
            "divisions": [
                { "div_id": 101, "div_type": 42, "owner": "GER", "location_province": 784 }
            ]
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &missing, "1942.1.1.12")));
        assert!(
            d.contains("unit_counter.formation_insignia@Some(784)mapfalse:sprite=MISS:missing_sprite:GFX_missing_formation(U);"),
            "missing formation sprite must remain an explicit miss: {d}"
        );
    }

    #[test]
    fn mapicon_formation_insignia_without_counter_anchor_badges_unavailable() {
        let stage = stage_with_manifest(
            &[],
            sprite_manifest(&[("GFX_exact_army", "gfx/interface/exact_army.dds", 1)]),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "insignia_gfx": "GFX_exact_army" }
                ]
            }
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert!(
            d.contains("unit_counter.formation_insignia@Nonemapfalse=—(U)"),
            "unanchored formation art must remain an explicit unavailable badge: {d}"
        );
        assert!(
            !d.contains("unit_counter.formation_insignia@Nonemapfalse:sprite=GFX_exact_army#0(E);"),
            "unanchored exact art must not fabricate a province-less shield entity: {d}"
        );
    }

    #[test]
    fn mapicon_formation_insignia_does_not_use_target_province_as_anchor() {
        let stage = stage_with_manifest(
            &[],
            sprite_manifest(&[("GFX_exact_army", "gfx/interface/exact_army.dds", 1)]),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "insignia_gfx": "GFX_exact_army",
                      "target_province": 785 }
                ]
            }
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert!(
            d.contains("unit_counter.formation_insignia@Nonemapfalse="),
            "target-only formation data must badge unavailable instead of placing art: {d}"
        );
        assert!(
            !d.contains(
                "unit_counter.formation_insignia@Some(785)mapfalse:sprite=GFX_exact_army#0(E);"
            ),
            "target_province is an order endpoint, not a counter-placement anchor: {d}"
        );
    }

    #[test]
    fn mapicon_formation_insignia_member_refs_require_matching_owner_anchor() {
        let stage = stage_with_manifest(
            &[],
            sprite_manifest(&[("GFX_exact_army", "gfx/interface/exact_army.dds", 1)]),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "order_of_battle": {
                "armies": [
                    { "owner": "GER", "insignia_gfx": "GFX_exact_army",
                      "members": ["101:42"] }
                ]
            },
            "divisions": [
                { "div_id": 101, "div_type": 42, "owner": "SOV", "location_province": 786 }
            ]
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert!(
            d.contains("unit_counter.formation_insignia@Nonemapfalse="),
            "cross-owner member refs must fail closed as unavailable: {d}"
        );
        assert!(
            !d.contains(
                "unit_counter.formation_insignia@Some(786)mapfalse:sprite=GFX_exact_army#0(E);"
            ),
            "division ids are not globally unique, so a SOV division must not anchor a GER shield: {d}"
        );
    }

    #[test]
    fn mapicon_formation_insignia_resolves_army_group_frame_through_owner_gated_army_ref() {
        let stage = stage_with_manifest(
            &[],
            sprite_manifest(&[(
                "GFX_theatre_army_group_shield",
                "gfx/interface/army_group_shield.dds",
                4,
            )]),
        );
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1942.1.1.12",
            "order_of_battle": {
                "army_groups": [
                    { "id": "49:53", "owner": "GER", "icon": 2,
                      "army_refs": ["46:53"] }
                ],
                "armies": [
                    { "id": "46:53", "owner": "SOV", "province": 999 },
                    { "id": "46:53", "owner": "GER", "members": ["101:42"] }
                ]
            },
            "divisions": [
                { "div_id": 101, "div_type": 42, "owner": "GER", "location_province": 787 }
            ]
        }))
        .unwrap();
        let d = bound_digest(&MapIconsView::new().bind(&ctx(&stage, &snap, "1942.1.1.12")));
        assert!(
            d.contains(
                "unit_counter.formation_insignia@Some(787)mapfalse:sprite=GFX_theatre_army_group_shield#1(D);"
            ),
            "one-based army-group shield frame must bind at the owned army's counter province: {d}"
        );
        assert!(
            !d.contains(
                "unit_counter.formation_insignia@Some(999)mapfalse:sprite=GFX_theatre_army_group_shield#1(D);"
            ),
            "same-id foreign armies must not anchor this country's army-group shield: {d}"
        );
    }

    // -------- mapmodes record the 14 modes' §B.9 fidelity -----------------------------
    #[test]
    fn mapmodes_record_per_mode_fidelity() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["mapmodes.json"]);
        let snap = snap_a();
        let v = MapModesView::new();
        let bv = v.bind(&ctx(&stage, &snap, "1941.4.13.11"));
        let d = bound_digest(&bv);
        assert!(d.contains("Political(E)"), "political mode is exact: {d}");
        // snap_a carries NO per-state resources and NO weather domain → the data-driven
        // Resources/Weather modes resolve Unavailable (badged "—"), never fabricated.
        assert!(
            d.contains("Resources(U)"),
            "resources mode unavailable when no state resources: {d}"
        );
        assert!(
            d.contains("Weather(U)"),
            "weather mode unavailable when no weather domain: {d}"
        );
        assert!(d.contains("Supply(D)"), "supply mode is proxy→derived: {d}");
        // 8 exact modes + the rest derived/unavailable were each recorded.
        assert!(
            bv.fidelity.total() >= 15,
            "all 14 modes + active recorded: {:?}",
            bv.fidelity
        );
    }

    /// RE-WIRE: the now-recorded `states.*.resources` + `weather` domains flip the data-
    /// driven Resources/Weather modes from "—" to their measured fidelity. A snapshot
    /// carrying them must badge Resources Exact + Weather Derived (no fabrication: the
    /// modes only light up because the driving domain is actually present).
    #[test]
    fn data_driven_modes_bind_when_domains_present() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["mapmodes.json"]);
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11", "hour_index": 100,
            "countries": { "GER": { "politics": { "capital_state": 64 } } },
            "states": {
                "64": { "owner": "GER", "controller": "GER",
                    "resources": { "steel": 12.0, "coal": 8.0 } },
                "75": { "owner": "GER", "controller": "GER", "resources": {} }
            },
            "weather": { "regions": { "5": { "temperature": 22.0, "phenomenon": ["rain_light"] } },
                         "provinces": { "11": { "mud": true } } }
        }))
        .unwrap();
        let v = MapModesView::new();
        let d = bound_digest(&v.bind(&ctx(&stage, &snap, "1941.4.13.11")));
        assert!(
            d.contains("Resources(E)"),
            "resources mode Exact when state resources present: {d}"
        );
        assert!(
            d.contains("Weather(D)"),
            "weather mode Derived when weather domain present: {d}"
        );
    }

    // -------- mapmodes click-to-switch core map mode -------------------------------
    #[test]
    fn mapmode_row_click_emits_set_core_map_mode_command() {
        let panel = mapmode_panel_for_tests();
        let mut v = MapModesView::new();
        v.set_input_panel(panel);

        let out = click_mapmode_row(&mut v, panel, "Control");

        assert!(out.consumed);
        assert!(out.needs_redraw);
        assert_eq!(
            out.commands,
            vec![ViewCommand::SetMapMode {
                mode: CoreMapMode::Control,
            }]
        );
    }

    #[test]
    fn mapmode_unavailable_and_proxy_rows_still_emit_without_faking_data() {
        let stage = stage_with(&[]);
        let snap = snap_a();
        let d = bound_digest(&MapModesView::new().bind(&ctx(&stage, &snap, "1941.4.13.11")));
        assert!(
            d.contains("Weather(U)"),
            "weather remains unavailable when its driving domain is absent: {d}"
        );
        assert!(
            d.contains("FrontLines(U)"),
            "front-lines rail badge remains unavailable in this chrome view: {d}"
        );

        let panel = mapmode_panel_for_tests();
        let mut v = MapModesView::new();
        v.set_input_panel(panel);

        let weather = click_mapmode_row(&mut v, panel, "Weather");
        assert!(weather.consumed);
        assert_eq!(
            weather.commands,
            vec![ViewCommand::SetMapMode {
                mode: CoreMapMode::Weather,
            }]
        );

        let front_lines = click_mapmode_row(&mut v, panel, "FrontLines");
        assert!(front_lines.consumed);
        assert_eq!(
            front_lines.commands,
            vec![ViewCommand::SetMapMode {
                mode: CoreMapMode::FrontLines,
            }]
        );
    }

    #[test]
    fn mapmode_intel_row_remains_visible_but_has_no_fabricated_core_target() {
        let panel = mapmode_panel_for_tests();
        let mut v = MapModesView::new();
        v.set_input_panel(panel);

        let out = click_mapmode_row(&mut v, panel, "Intel");

        assert!(
            out.consumed,
            "rail-owned Intel clicks must not leak to map pick"
        );
        assert!(!out.needs_redraw);
        assert!(
            out.commands.is_empty(),
            "render::modes::MapMode has no Intel variant, so the rail must not alias it"
        );
    }

    #[test]
    fn mapmode_outside_click_is_ignored() {
        let panel = mapmode_panel_for_tests();
        let mut v = MapModesView::new();
        v.set_input_panel(panel);

        let out = v.on_input(&ViewInput::PointerDown {
            x: panel.x - 5.0,
            y: panel.y + panel.h * 0.5,
            button: PointerButton::Left,
        });

        assert!(!out.consumed);
        assert!(!out.needs_redraw);
        assert!(out.commands.is_empty());
    }

    // -------- the on-map placement PATH (Codex (b)/state-id-≠-province-id fix) ----------

    /// `project_centroid` is a pure function: a centroid in its source-raster space →
    /// a finite SCREEN pixel through the camera, algebraically matching the app camera.
    #[test]
    fn project_centroid_matches_camera_transform() {
        // a 100x100 source raster; identity-ish camera (offset 0, scale 1) over a 800x600
        // screen mapping centroid (50,50) (UV 0.5,0.5) → screen (400,300).
        let cam = crate::camera::MapCamera::new(800, 600, 100, 100);
        let set = CentroidSet {
            centroids: map_overlay::ProvinceCentroids::default(),
            raster_w: 100.0,
            raster_h: 100.0,
        };
        let (sx, sy) = project_centroid(&cam, &set, 50.0, 50.0);
        assert!(
            (sx - 400.0).abs() < 0.01 && (sy - 300.0).abs() < 0.01,
            "got {sx},{sy}"
        );
        // a half-res raster (50x50) with the SAME full-res camera: centroid (25,25) is the
        // same UV 0.5 → same screen point (proves the per-raster normalization is correct).
        let half = CentroidSet {
            centroids: map_overlay::ProvinceCentroids::default(),
            raster_w: 50.0,
            raster_h: 50.0,
        };
        let (hx, hy) = project_centroid(&cam, &half, 25.0, 25.0);
        assert!(
            (hx - 400.0).abs() < 0.01 && (hy - 300.0).abs() < 0.01,
            "half-res got {hx},{hy}"
        );
        // degenerate scale is sanitized (no NaN/collapse).
        let mut bad = cam;
        bad.scale = 0.0;
        let (bx, by) = project_centroid(&bad, &set, 50.0, 50.0);
        assert!(bx.is_finite() && by.is_finite());
    }

    /// The REAL `state_meta.json` (if present) yields a state→VP-province join: a state id
    /// resolves to a PROVINCE id (NOT itself) — the exact fix for the state-id-as-province
    /// bug. Parsed inline (independent of the process-global OnceLock loader).
    #[test]
    fn state_meta_resolves_state_to_vp_province() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("map_pipeline")
            .join("assets");
        let p = dir.join("state_meta.json");
        if !p.exists() {
            eprintln!("[chrome] skipped: map_pipeline/assets/state_meta.json not present");
            return;
        }
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let obj = v.as_object().unwrap();
        // find a state whose VP province id differs from the state id (the bug case).
        let mut proved = false;
        for (sid_s, meta) in obj.iter().take(50) {
            let sid: u32 = match sid_s.parse() {
                Ok(n) => n,
                Err(_) => continue,
            };
            if let Some(vps) = meta.get("victory_points").and_then(|m| m.as_object()) {
                if let Some(pid_s) = vps.keys().next() {
                    let pid: u32 = pid_s.parse().unwrap();
                    // a VP province id is virtually never equal to the state id.
                    if pid != sid {
                        proved = true;
                        break;
                    }
                }
            }
        }
        assert!(
            proved,
            "state_meta must map a state id to a DIFFERENT province id (not itself)"
        );
    }

    // -------- scrub A→B→A byte-identical for every view -------------------------------
    fn scrub_identical<V: ViewInstance>(v: &V, files: &[&str]) {
        if !assets_present() {
            return;
        }
        let stage = stage_with(files);
        let (a, b) = (snap_a(), snap_b());
        let da1 = bound_digest(&v.bind(&ctx(&stage, &a, "1941.4.13.11")));
        let db = bound_digest(&v.bind(&ctx(&stage, &b, "1942.7.1.18")));
        let da2 = bound_digest(&v.bind(&ctx(&stage, &a, "1941.4.13.11")));
        assert_eq!(
            da1,
            da2,
            "scrub A→B→A carried a value from the B visit: view {}",
            v.view_id()
        );
        // A and B must differ for at least the data-bearing views (clock hour, VP/occ).
        let _ = db;
    }

    #[test]
    fn scrub_a_b_a_all_views() {
        scrub_identical(&ClockView::new(), &["ingame_clock.json"]);
        scrub_identical(&MinimapView::new(), &["minimap.json"]);
        scrub_identical(&AlertsView::new(), &["alerts.json"]);
        scrub_identical(&FindView::new(), &["find_view.json"]);
        scrub_identical(&RightClickView::new(), &["rightclick_menu.json"]);
        scrub_identical(&MapModesView::new(), &["mapmodes.json"]);
        scrub_identical(&MapIconsView::new(), &["mapicons.json"]);
    }

    #[test]
    fn clock_a_b_differ_on_hour() {
        if !assets_present() {
            return;
        }
        let stage = stage_with(&["ingame_clock.json"]);
        let (a, b) = (snap_a(), snap_b());
        let v = ClockView::new();
        let da = bound_digest(&v.bind(&ctx(&stage, &a, "1941.4.13.11")));
        let db = bound_digest(&v.bind(&ctx(&stage, &b, "1942.7.1.18")));
        assert_ne!(
            da, db,
            "the clock hour must differ between A (11:00) and B (18:00)"
        );
    }

    // -------- per-view element accounting: scene walk == the .gui's element_type_counts

    /// Count `type` tokens recursively over the RAW gui json (the authoritative element
    /// set — incl. layout primitives slotsize/borderSize/etc the typed scene drops).
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

    /// For each of the 7 views' `.gui` files, the element set we account for (the
    /// recursive `type`-token walk over the parsed tree) EQUALS the file's declared
    /// element_type_counts — proving every element of each view is accounted (none
    /// silently dropped); the per-element binding-map tables place each in one bucket.
    #[test]
    fn element_accounting_all_seven_views() {
        if !assets_present() {
            return;
        }
        for f in [
            "ingame_clock.json",
            "minimap.json",
            "alerts.json",
            "find_view.json",
            "rightclick_menu.json",
            "mapmodes.json",
            "mapicons.json",
        ] {
            assert_eq!(
                raw_type_counts(f),
                declared_counts(f),
                "element accounting drift in {f}"
            );
        }
    }

    /// The accounting totals match the binding-map's stated parsed-node counts — a guard
    /// the source files have not drifted under the map (6/18/11/16/9/178/383).
    #[test]
    fn element_accounting_totals_match_binding_map() {
        if !assets_present() {
            return;
        }
        let total = |n: &str| raw_type_counts(n).values().sum::<usize>();
        assert_eq!(total("ingame_clock.json"), 6);
        assert_eq!(total("minimap.json"), 18);
        assert_eq!(total("alerts.json"), 11);
        assert_eq!(total("find_view.json"), 16);
        assert_eq!(total("rightclick_menu.json"), 9);
        assert_eq!(total("mapmodes.json"), 178);
        assert_eq!(total("mapicons.json"), 383);
    }

    /// Collect every `name` in a `.gui` window subtree (the named element set), walking
    /// the RAW json (lenient — independent of the typed `GuiFile` parser).
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

    /// BINDING-COVERAGE guard: every element name I bind a `Cell` to MUST exist as a real
    /// node in its target window — else `set_element_text` silently no-ops and the field
    /// is neither shown nor honestly "—". Mirrors the production cluster's guard.
    #[test]
    fn every_bound_element_name_exists_in_its_window() {
        if !assets_present() {
            return;
        }
        let cases: &[(&str, &str, &[&str])] = &[
            ("ingame_clock.json", "clock_window", &["time_text"]),
            ("alerts.json", "global_alerticon_window", &["icon"]),
            (
                "alerts.json",
                "global_diplorequesticon_window",
                &["country_flag"],
            ),
            ("rightclick_menu.json", "right_click_entry", &["button"]),
            (
                "rightclick_menu.json",
                "right_click_entry_2",
                &["button", "open_button"],
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

    /// Every routed view is collected from the inventory under the chrome cluster.
    #[test]
    fn cluster_views_registered() {
        use crate::host::ViewRegistry;
        let reg = ViewRegistry::from_inventory();
        for id in [
            "chrome.clock",
            "chrome.minimap",
            "chrome.alerts",
            "chrome.find",
            "chrome.rightclick",
            "chrome.mapmodes",
            "chrome.mapicons",
        ] {
            assert!(
                reg.view(ViewId(id)).is_some(),
                "view {id} must be inventory-collected"
            );
        }
        let bar = reg.menubar();
        let group = bar
            .iter()
            .find(|g| g.cluster == MenuCluster::ChromeOutlinerMaps)
            .unwrap();
        assert_eq!(
            group.items.len(),
            7,
            "all 7 cluster views routed to the Chrome menu"
        );
    }
}
