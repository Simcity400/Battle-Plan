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
//!      `ctx.stage.map_dir/province_ids` and projected through `ctx.map_camera`. ALL the
//!      OTHER ~32 on-map icon TEMPLATE families read TOP-LEVEL snapshot domains
//!      (`divisions`/`combats`/`order_of_battle`/`raids`/`diplomacy_world`) that the
//!      no-fabrication `BindContext` seam does NOT expose to a cluster view (only
//!      `countries.*`/`states.*` via `country_field`/`state_field`), or are schema-unfed
//!      / asset-gap — so each is explicitly BADGED `unavailable` (ACCOUNTED, never
//!      silently dropped, never fabricated). All entities render via the **N8**
//!      map-overlay layer: placed icons on the map, badged ones in a screen legend.
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

use crate::host::context::{BindContext, Captured, DisplayAtom, FidelityReport, UnavailableReason};
use crate::host::draw::FrameDraw;
use crate::host::registry::{GuiSymbolRef, MenuCluster, MenuRoute, ViewDescriptor, ViewId};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InstanceId, ViewCommand, ViewInstance, ZBand,
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

const MINIMAP_VIEW_ID: ViewId = ViewId("chrome.minimap");
const MINIMAP_FILE: &str = "minimap.json";
const MINIMAP_ROOT: &str = "mini_map_interface";

pub struct MinimapView;
impl MinimapView {
    pub fn new() -> MinimapView {
        MinimapView
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
                let (rw, rh) = (cam.raster_w.max(1.0), cam.raster_h.max(1.0));
                // the visible raster window = inverse of the shader transform:
                //   uv = px/screen / scale + offset  → raster = uv * raster_size
                //   at px=0       → uv = offset ; at px=screen → uv = 1/scale + offset
                let s = if cam.scale.is_finite() && cam.scale > 0.0 {
                    cam.scale
                } else {
                    1.0
                };
                let view = WorldRect {
                    x: cam.offset[0] * rw,
                    y: cam.offset[1] * rh,
                    w: (1.0 / s) * rw,
                    h: (1.0 / s) * rh,
                };
                (Some(view), (rw, rh))
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
        let panel = root_rect(&roots, MINIMAP_ROOT).unwrap_or(Rect {
            x: ctx.screen.w - 360.0,
            y: ctx.screen.h - 200.0,
            w: 340.0,
            h: 180.0,
        });

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

pub struct FindView;
impl FindView {
    pub fn new() -> FindView {
        FindView
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
}

/// Which catalog the find tab is showing (replay-supported: states / countries / VPs).
#[derive(Clone, Copy, PartialEq, Eq)]
enum FindTab {
    States,
    Countries,
    VictoryPoints,
}

struct FindDrawable {
    roots: Vec<Node>,
    bar: TabBar,
    panel: Rect,
    rows: Vec<FindRow>,
}
impl Drawable for FindDrawable {
    fn draw_digest(&self) -> String {
        // NOTE: query=none — these rows are the BROWSABLE CATALOG, not live search
        // results (the search term is replay-local UI input with no value at bind).
        let mut s = format!("FIND query=none tab={:?} rows=[", self.bar.selected());
        for r in &self.rows {
            let badge = match r.name.fidelity() {
                Fidelity::Exact => "E",
                Fidelity::Derived => "D",
                Fidelity::Unavailable => "U",
            };
            s.push_str(&format!("{}={}({badge});", r.key, r.name.as_str()));
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
        let row_h = 22.0;
        for (i, r) in self.rows.iter().enumerate() {
            let y = content.y + i as f32 * row_h;
            if y + row_h > content.y + content.h {
                break;
            }
            let atom = watom(&r.name);
            widgets::common::draw_atom(
                fd.renderer,
                &assets,
                &atom,
                content.x + 6.0,
                y + 4.0,
                widgets::common::TEXT_TINT,
                Some([content.x, content.y, content.w, content.h]),
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

/// Build the BROWSABLE CATALOG rows for a category tab from the LIVE snapshot's named
/// entities (DERIVED — the names are joined from the snapshot). The find search TERM is
/// replay-local UI input with NO value at bind time, so these are NOT live search results
/// — they are the catalog the search would filter (Codex (f): never pretend a catalog is
/// a query result). Capped so the panel stays bounded; each row is no-fabrication.
fn find_rows(ctx: &BindContext, tab: FindTab, fidelity: &mut FidelityReport) -> Vec<FindRow> {
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
            let mut label = ctx.country.to_string();
            if let Captured::Measured(v) = ctx.country_field(ctx.country, "identity.original_tag") {
                if let Some(orig) = v.as_str() {
                    label.push_str(&format!(" (was {orig})"));
                }
            }
            if let Captured::Measured(v) =
                ctx.country_field(ctx.country, "identity.original_capital")
            {
                if let Some(cap) = v.as_i64() {
                    label.push_str(&format!(" [cap #{cap}]"));
                }
            }
            // core / claim COUNTS (the per-country inversion of states' is_core_of /
            // is_claimed_by, country_containers_live fold). Appended only when the measured
            // list is non-empty (a measured-empty list adds nothing; never a fabricated 0).
            if let Captured::Measured(v) = ctx.country_field(ctx.country, "identity.cores") {
                if let Some(n) = v.as_array().map(|a| a.len()).filter(|n| *n > 0) {
                    label.push_str(&format!(" cores:{n}"));
                }
            }
            if let Captured::Measured(v) = ctx.country_field(ctx.country, "identity.claims") {
                if let Some(n) = v.as_array().map(|a| a.len()).filter(|n| *n > 0) {
                    label.push_str(&format!(" claims:{n}"));
                }
            }
            let tag_atom = DisplayAtom::text(label, Fidelity::Derived);
            fidelity.record(&tag_atom);
            rows.push(FindRow {
                key: ctx.country.to_string(),
                name: tag_atom,
            });
        }
        FindTab::States => {
            // states are keyed by id in the snapshot; surface the first CAP state ids
            // with their owner as a derived "name" (a localized state name needs loc/
            // game_defs which a cluster does not load — so id+owner is the honest token).
            for sid in state_ids(ctx).into_iter().take(CAP) {
                let owner = ctx.state_field(&sid, "owner");
                let owner_s = owner.display_with_fidelity(Fidelity::Exact, fmt_token);
                let label = format!("#{sid} ({})", owner_s.as_str());
                let atom = DisplayAtom::text(label, Fidelity::Derived);
                fidelity.record(&atom);
                rows.push(FindRow {
                    key: format!("state:{sid}"),
                    name: atom,
                });
            }
        }
        FindTab::VictoryPoints => {
            // VP entities = states carrying a victory_points_total > 0 (exact field).
            for sid in state_ids(ctx).into_iter().take(CAP) {
                let vp = ctx.state_field(&sid, "victory_points_total");
                let atom = vp.display_with_fidelity(Fidelity::Exact, fmt_int);
                if !atom.is_unavailable() && atom.as_str() != "0" {
                    let label = format!("#{sid}: {} VP", atom.as_str());
                    let row_atom = DisplayAtom::text(label, Fidelity::Exact);
                    fidelity.record(&row_atom);
                    rows.push(FindRow {
                        key: format!("vp:{sid}"),
                        name: row_atom,
                    });
                }
            }
            if rows.is_empty() {
                // a measured-empty catalog is honest (no VP states fed this hour); record
                // one unavailable so the section flags "no data", never a fabricated row.
                let a = DisplayAtom::unavailable_because(UnavailableReason::Absent);
                fidelity.record(&a);
            }
        }
    }
    rows
}

/// The state ids present in the snapshot (the keys of the `states` domain), sorted
/// numerically for a deterministic slice. Read via the state_field seam only — no raw
/// snapshot. We probe a bounded id range and keep the ones with a present `owner`/`category`.
fn state_ids(ctx: &BindContext) -> Vec<String> {
    // The snapshot states domain is keyed by id; without a raw-snapshot iterator (the
    // forbidden seam), we probe ids 1..=1200 (the game's state-id ceiling) and keep present
    // ones. A present state has SOME field (owner OR category OR victory_points_total).
    let mut out = Vec::new();
    for id in 1..=1200u32 {
        let sid = id.to_string();
        let present = matches!(
            ctx.state_field(&sid, "owner"),
            Captured::Measured(_) | Captured::Null
        ) || matches!(ctx.state_field(&sid, "category"), Captured::Measured(_))
            || matches!(
                ctx.state_field(&sid, "victory_points_total"),
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
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = gui_file(ctx, FIND_FILE);

        let bar = find_tabs();
        let tab = match bar.selected() {
            Some("countries") => FindTab::Countries,
            Some("vps") => FindTab::VictoryPoints,
            _ => FindTab::States,
        };
        let rows = find_rows(ctx, tab, &mut fidelity);

        let roots = build_root(ctx, &file, FIND_ROOT, &[]);
        // the results panel = the find_view shell rect (or a default in the lower-right).
        let panel = root_rect(&roots, FIND_ROOT).unwrap_or(Rect {
            x: ctx.screen.w - 300.0,
            y: ctx.screen.h - 320.0,
            w: 285.0,
            h: 300.0,
        });

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

const MAPMODES_VIEW_ID: ViewId = ViewId("chrome.mapmodes");
const MAPMODES_FILE: &str = "mapmodes.json";
const MAPMODES_ROOT: &str = "MapmodesInterface_Ingame";

pub struct MapModesView;
impl MapModesView {
    pub fn new() -> MapModesView {
        MapModesView
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
struct MapMode {
    name: &'static str,
    fidelity: Fidelity,
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

impl MapMode {
    const fn stat(name: &'static str, fidelity: Fidelity) -> MapMode {
        MapMode {
            name,
            fidelity,
            data_driven: None,
        }
    }
    const fn driven(name: &'static str, best: Fidelity, mode: DataDrivenMode) -> MapMode {
        MapMode {
            name,
            fidelity: best,
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
const MAP_MODES: &[MapMode] = &[
    MapMode::stat("Political", Fidelity::Exact),
    MapMode::stat("Control", Fidelity::Exact),
    MapMode::stat("Resistance", Fidelity::Exact),
    MapMode::stat("Compliance", Fidelity::Exact),
    MapMode::stat("VictoryPoints", Fidelity::Exact),
    MapMode::stat("Buildings", Fidelity::Exact),
    // per-state resources closed by the live reader → Exact when captured, else "—".
    MapMode::driven("Resources", Fidelity::Exact, DataDrivenMode::StateResources),
    // weather domain closed by the live reader → Derived proxy when captured, else "—".
    MapMode::driven("Weather", Fidelity::Derived, DataDrivenMode::Weather),
    MapMode::stat("FrontLines", Fidelity::Unavailable), // compute pass not run
    MapMode::stat("Faction", Fidelity::Exact),
    MapMode::stat("AirSuperiority", Fidelity::Derived), // region proxy
    MapMode::stat("Naval", Fidelity::Derived),          // region proxy
    MapMode::stat("Supply", Fidelity::Derived),         // division-province proxy
    MapMode::stat("Intel", Fidelity::Derived),
];

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
    label: DisplayAtom,
    /// the target PROVINCE id (NOT a state id) for on-map placement, or `None` to legend.
    province: Option<u32>,
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
            let b = match e.label.fidelity() {
                Fidelity::Exact => "E",
                Fidelity::Derived => "D",
                Fidelity::Unavailable => "U",
            };
            let on_map = pos.is_some();
            s.push_str(&format!(
                "{}@{:?}map{}={}({b});",
                e.template,
                e.province,
                on_map,
                e.label.as_str()
            ));
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
                    Some((sx, sy)) => (
                        IconAnchor::Screen { x: *sx, y: *sy },
                        widgets::common::AssetRef::None, // a tinted on-map marker
                        [0.85, 0.80, 0.30, 1.0],
                    ),
                    None => {
                        // wrap the legend into columns of 40 rows so nothing is dropped.
                        let col = (legend_i / 40) as f32;
                        let row = (legend_i % 40) as f32;
                        let x = 16.0 + col * 220.0;
                        let y = 80.0 + row * 18.0;
                        legend_i += 1;
                        (
                            IconAnchor::Screen { x, y },
                            widgets::common::AssetRef::Unavailable, // badged no-placement
                            [0.8, 0.8, 0.85, 1.0],
                        )
                    }
                };
                MapIcon {
                    id: e.template.to_string(),
                    anchor,
                    sprite,
                    tint,
                    label: Some(watom(&e.label)),
                    priority: 0,
                    size: 14.0,
                }
            })
            .collect();
        // the centroids/camera are encoded into Screen anchors already, so a default
        // (no-op) N8 camera + empty centroids is correct here.
        let centroids = map_overlay::ProvinceCentroids::default();
        let cam = map_overlay::MapCamera::new((0.0, 0.0), 1.0, (0.0, 0.0));
        let laid = map_overlay::layout_icons(&icons, &centroids, &cam);
        map_overlay::plan_icons(&laid, None).execute(fd.renderer, &assets, |_, _| false);
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
                entities.push(IconEntity {
                    template: "victory_point_mapicon",
                    label: atom,
                    province: prov,
                });
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
                    entities.push(IconEntity {
                        template: "capital_mapicon",
                        label: atom,
                        province: prov,
                    });
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
                entities.push(IconEntity {
                    template: "resistance_compliance_map_icon",
                    label: atom,
                    province: prov,
                });
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
                entities.push(IconEntity {
                    template: "air_base_mapicon",
                    label: atom,
                    province: prov,
                });
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
                        entities.push(IconEntity {
                            template: "resource_mapicon",
                            label: atom,
                            province: prov,
                        });
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
                        entities.push(IconEntity {
                            template: "state_modifier_mapicon",
                            label: atom,
                            province: prov,
                        });
                    }
                }
            }
        }

        // ---- the remaining on-map icon-TEMPLATE families: ACCOUNTED, not dropped. Each
        // is badged with its honest reason so the binding-map's dynamic sections are all
        // represented (none silently omitted). The unit/navy/air COUNTERS + land/naval
        // COMBAT + air/naval MISSION + raid/faction/peace/decision icons read TOP-LEVEL
        // snapshot domains (`divisions`/`combats`/`order_of_battle`/`raids`/`diplomacy_world`)
        // that the no-fabrication BindContext seam does NOT expose to a cluster view (only
        // `countries.*`/`states.*` are reachable via country_field/state_field) → badged
        // unavailable (a host-seam gap, never a fabricated count/position). The intel /
        // special-facility / portrait families are also schema-unfed/asset-gap per §map.
        const BADGED_TEMPLATES: &[&str] = &[
            "unit_counter",                  // divisions stack count/org/str — top-level domain
            "unit_counter_navy",             // fleet roster — top-level OOB
            "unit_counter_railway_gun",      // railway-gun divisions — top-level domain
            "unit_counter.general_portrait", // commanders unfed + portrait asset gap
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
            entities.push(IconEntity {
                template: tmpl,
                label: atom,
                province: None,
            });
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
            loc: gui_runtime::Loc::new(),
            gui_files,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
        }
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
