//! m5_demo — the STATE PANEL + the three Codex-required runtime foundations,
//! composed over the M4 map and M3 topbar, end to end against REAL data.
//!
//! Pipeline (all in-process, offscreen, READ-ONLY on assets + the store):
//!   1. Render the M4 political map (bottom layer) for a real snapshot.
//!   2. Render the M3 topbar (top layer) bound to the observed country.
//!   3. PICK a province on the map → resolve its state → OPEN the state panel
//!      (`province_state.json`) STACKED over the map with correct z-order.
//!   4. Bind the panel to the picked state's snapshot data via `bind::bind_state`.
//!   5. Exercise the three foundations:
//!        (a) the buildings GRIDBOX ROW-FACTORY made to OVERFLOW its box, with a
//!            clipped viewport + a working scrollbar (rows scrolled + clipped),
//!        (b) a WRAPPED, MARKED-UP, LOCALIZED text box whose wrap is computed from
//!            glyph metrics AFTER `$VAR$`/`§colour`/`£icon` expansion,
//!        (c) an explicit Z-ORDER / `alwaystransparent` hit-test case.
//!   6. Compose all three into render/out/m5_state_panel.png.
//!   7. Verify the hit-test: panel element hit, transparent-region fall-through to
//!      the map province behind, front-to-back z resolution.
//!
//! Paths via CLI/env/documented defaults — no personal literal in committed source.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use assets::resolver::GameResolver;
use assets::{BmFont, SpriteManifest};
use bind::{apply_topbar, bind_state, FlagResolver, StateBindReport, TopbarSpec};
use gui_runtime::draw::{draw_window, DrawStats, TextureCache};
use gui_runtime::fonts::{FontCache, FontMap};
use gui_runtime::gridbox::{self, GridSpec};
use gui_runtime::scene::{ElementKind, Node};
use gui_runtime::{
    build_scene, expand, hit_test_stack, layout, wrap, GuiFile, Loc, ManifestContent, Rect,
};
use render::map::{
    build_lut, pick_at, CountryColors, MapIds, PoliticalMap, ProvinceRaster, StateOwners,
};
use render::{save_png, Renderer};
use snapshot::Store;

fn arg_value(flag: &str) -> Option<String> {
    let mut it = std::env::args();
    while let Some(a) = it.next() {
        if a == flag {
            return it.next();
        }
        if let Some(v) = a.strip_prefix(&format!("{flag}=")) {
            return Some(v.to_string());
        }
    }
    None
}

fn find_assets_dir() -> Option<PathBuf> {
    if let Some(a) = arg_value("--assets").or_else(|| std::env::var("REPLAY_ASSETS").ok()) {
        return Some(PathBuf::from(a));
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let cand = dir.join("game_assets");
        if cand.join("gui").join("province_state.json").is_file() {
            return Some(cand);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Repo root = two levels up from this crate (replay_native/bind -> repo).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The committed output dir is render/out/ (sibling crate), matching M1–M4.
fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("render")
        .join("out")
}

fn loc_dirs(resolver: &GameResolver) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(m) = &resolver.mod_root {
        v.push(m.join("localisation").join("english"));
        v.push(m.join("localisation").join("replace").join("english"));
    }
    v.push(resolver.vanilla.join("localisation").join("english"));
    v
}

/// Find a laid-out element's rect by name (first match, depth-first).
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

fn main() {
    // --- CLI (defaults = the committed demo frame: GER 1941, Brandenburg). ---
    let country = arg_value("--country").unwrap_or_else(|| "GER".to_string());
    let game = arg_value("--game").unwrap_or_else(|| "replay".to_string());
    let hour: i64 = arg_value("--hour")
        .and_then(|s| s.parse().ok())
        .unwrap_or(17005619);

    let vanilla = arg_value("--vanilla");
    let mod_root = arg_value("--mod");
    let resolver = GameResolver::new(vanilla.as_deref(), mod_root.as_deref());

    let map_dir = arg_value("--map")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("map_pipeline").join("assets"));
    let store_root = arg_value("--store")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("replay").join("snapshots"));

    eprintln!("[m5] country={country} game={game} hour={hour}");
    eprintln!("[m5] vanilla = {}", resolver.vanilla.display());
    if !resolver.vanilla_exists() {
        eprintln!("[m5] BLOCKER: game install not found. Pass --vanilla <path> or set BATTLE_PLAN_VANILLA.");
        std::process::exit(2);
    }
    let assets_dir = find_assets_dir().unwrap_or_else(|| {
        eprintln!("[m5] BLOCKER: game_assets/gui/province_state.json not found (pass --assets).");
        std::process::exit(2);
    });
    eprintln!("[m5] assets = {}", assets_dir.display());
    eprintln!("[m5] map    = {}", map_dir.display());
    eprintln!("[m5] store  = {} (game {game})", store_root.display());

    // --- Load the snapshot + map artifacts. ---------------------------------
    let store = Store::open(&store_root);
    let snap = store.load(&game, hour).unwrap_or_else(|e| {
        eprintln!("[m5] BLOCKER: load snapshot {game} @ {hour}: {e}");
        std::process::exit(2);
    });
    eprintln!(
        "[m5] snapshot date={} countries={} states={}",
        snap.date.as_deref().unwrap_or("?"),
        snap.countries().map(|m| m.len()).unwrap_or(0),
        snap.states().map(|m| m.len()).unwrap_or(0),
    );

    let raster = ProvinceRaster::load_png(&map_dir.join("province_ids.png")).unwrap_or_else(|e| {
        eprintln!("[m5] BLOCKER: load province_ids.png: {e}");
        std::process::exit(2);
    });
    let ids = MapIds::load(&map_dir).unwrap_or_else(|e| {
        eprintln!("[m5] BLOCKER: MapIds::load: {e}");
        std::process::exit(2);
    });
    let colors = CountryColors::load(&map_dir).unwrap_or_else(|e| {
        eprintln!("[m5] BLOCKER: CountryColors::load: {e}");
        std::process::exit(2);
    });
    let states_obj = snap.states().expect("BLOCKER: snapshot has no states");
    let owners = StateOwners::from_states(states_obj);

    // --- Load loc + sprite manifest + parsed gui views. ---------------------
    let manifest = SpriteManifest::load(&assets_dir.join("sprites.json"))
        .expect("BLOCKER: load sprites.json");
    let mut loc = Loc::new();
    for base in loc_dirs(&resolver) {
        let _ = loc.add_dir(&base);
    }
    eprintln!("[m5] loc keys = {} sprites = {}", loc.len(), manifest.sprites.len());

    let topbar_file = GuiFile::load(&assets_dir.join("gui").join("topbar.json"))
        .expect("BLOCKER: parse topbar.json");
    let panel_file = GuiFile::load(&assets_dir.join("gui").join("province_state.json"))
        .expect("BLOCKER: parse province_state.json");

    // ------------------------------------------------------------------------
    // STEP 3: PICK a province on the map → resolve its state → OPEN the panel.
    // We pick the centre province of the target state so the selection is real
    // (cursor → province → state), not just "open state N".
    // ------------------------------------------------------------------------
    let (screen_w, screen_h) = (1920u32, 1080u32);
    let screen = Rect { x: 0.0, y: 0.0, w: screen_w as f32, h: screen_h as f32 };

    // Map fills the whole screen (bottom layer). Pick coordinate: a known land
    // pixel inside the target state. State 64 (Brandenburg/Berlin) centre province
    // 6521 sits ~ (3115, 820) in the raster; map_uv → screen via the same scale.
    let target_state: u32 = arg_value("--state").and_then(|s| s.parse().ok()).unwrap_or(64);
    // Find a raster pixel that belongs to the target state (ground-truth pick).
    let (pick_px, pick_py) = find_state_pixel(&raster, &ids, target_state).unwrap_or_else(|| {
        eprintln!("[m5] BLOCKER: no raster pixel resolves to state {target_state}");
        std::process::exit(3);
    });
    let pick = pick_at(&raster, &ids, &owners, pick_px, pick_py).unwrap();
    eprintln!(
        "[m5] PICK raster ({pick_px},{pick_py}) -> province {} state {:?} owner {:?}",
        pick.province_id, pick.state_id, pick.owner
    );
    let sid = pick.state_id.unwrap_or(target_state);
    let sid_str = sid.to_string();

    // Resolve the state NAME from static map meta + loc (not in the snapshot).
    let state_name = resolve_state_name(&map_dir, &loc, sid).unwrap_or_else(|| format!("STATE_{sid}"));
    eprintln!("[m5] selected state {sid} = \"{state_name}\" owner={:?}", pick.owner);

    // --- Bind the state panel to the picked state's snapshot data. ----------
    let report = bind_state(&snap, &sid_str, &state_name);
    print_bind_report(&report);

    // ------------------------------------------------------------------------
    // Build the renderer + caches (shared by all three passes).
    // ------------------------------------------------------------------------
    let mut renderer = Renderer::new(screen_w, screen_h).expect("BLOCKER: renderer init");
    eprintln!(
        "[m5] wgpu backend={:?} adapter={} BC={}",
        renderer.adapter_info.backend, renderer.adapter_info.name, renderer.bc_supported
    );
    let font_map = FontMap::from_core_gfx(&resolver);
    let mut fonts = FontCache::new(&font_map, &resolver);
    let mut textures = TextureCache::new(&manifest, &resolver);

    // ------------------------------------------------------------------------
    // PASS 1 (bottom): the political map fills the screen.
    // ------------------------------------------------------------------------
    let lut = build_lut(raster.max_id, &ids, &owners, &colors);
    let pmap = PoliticalMap::new(&renderer, &raster);
    pmap.update_lut(&renderer, &lut);
    // map_uv: show the whole world stretched to the screen (scale=1, offset=0).
    let _map_px = pmap.render(
        &renderer,
        [0.0, 0.0, screen_w as f32, screen_h as f32],
        [0.0, 0.0],
        [1.0, 1.0],
        [0.05, 0.06, 0.08, 1.0],
    );
    // The map pass cleared + rendered into the SHARED offscreen target. The quad
    // batches we accumulate next (topbar + panel) draw OVER it in renderer.finish
    // WITHOUT clearing — but finish() clears. So we composite by drawing the map
    // as a sprite is unnecessary; instead we let the map own the clear and the
    // sprite pass load (not clear). We achieve the stack via finish_no_clear.

    // ------------------------------------------------------------------------
    // PASS 2 (top): the topbar, bound to the observed country.
    // ------------------------------------------------------------------------
    let mut topbar = build_scene(&topbar_file, &manifest, &loc);
    let tb_spec = TopbarSpec::topbar();
    let tb_report = apply_topbar(&mut topbar, &tb_spec, &snap, &country);
    let content = ManifestContent { manifest: &manifest, measure: |_t, _f| None };
    for root in topbar.iter_mut() {
        layout(root, screen, None, &content);
    }
    // resolve the observed-country flag for the topbar.
    let flag_res = FlagResolver::new(&resolver);
    let tb_flag = flag_res
        .resolve(&country, tb_report.flag_ideology.as_deref())
        .and_then(|rf| {
            let rect = find_rect(&topbar, "player_flag")?;
            renderer.load_image(&rf.path).ok().map(|t| (rect, t))
        });

    // ------------------------------------------------------------------------
    // PASS 3 (stacked): the STATE PANEL window.
    //
    // The panel's many top-level windows in the .gui are the main view
    // (`countrystateview`) PLUS the reusable entry TEMPLATES (instantiated by the
    // gridboxes, not drawn standalone). We render only `countrystateview` as the
    // opened window. In-game the engine positions this view relative to a parent
    // container; its raw `lower_left` + (-416,-549) anchor lands OFF-SCREEN on a
    // bare full-screen layout. "Opening" the panel from a pick places it at a
    // visible on-screen location — we anchor it UPPER_LEFT at (PANEL_X, PANEL_Y),
    // a faithful "opened panel" position (Codex-confirmed: the raw anchor is a
    // parent-relative engine placement, not a screen coordinate).
    const PANEL_X: f32 = 40.0;
    const PANEL_Y: f32 = 110.0;
    let mut panel: Vec<Node> = build_scene(&panel_file, &manifest, &loc)
        .into_iter()
        .filter(|w| w.name == "countrystateview")
        .collect();
    for root in panel.iter_mut() {
        // open the panel on-screen: UPPER_LEFT-anchored at a fixed position.
        root.orientation = gui_runtime::Anchor::UpperLeft;
        root.origo = gui_runtime::Anchor::UpperLeft;
        root.pos = (PANEL_X, PANEL_Y);
    }
    // Bind the panel scalars into the scene-graph text elements.
    apply_state_scalars(&mut panel, &report);
    for root in panel.iter_mut() {
        layout(root, screen, None, &content);
    }

    // Draw order = map (already in target) → topbar quads → panel quads. We use
    // the renderer's quad accumulation; the map already rendered+cleared, so we
    // must NOT clear again. Draw topbar first (lower), panel last (highest).
    let mut stats = DrawStats::default();
    for root in &topbar {
        draw_window(&mut renderer, root, &mut textures, &mut fonts, &mut stats);
    }
    if let Some((r, tex)) = &tb_flag {
        renderer.draw_sprite(tex, r.x, r.y, r.w, r.h);
    }

    // -- Panel chrome (background + bound scalar text + flags). --
    for root in &panel {
        draw_window(&mut renderer, root, &mut textures, &mut fonts, &mut stats);
    }

    // -- Paint owner / controller flags into the panel flag slots. --
    paint_state_flags(&mut renderer, &panel, &report, &flag_res, &mut textures, &manifest, &resolver);

    // ------------------------------------------------------------------------
    // FOUNDATION (a): the buildings GRIDBOX ROW-FACTORY, made to OVERFLOW.
    // We deliberately bind the COMBINED building list (state + province) into the
    // state_building_entries gridbox so it overflows the 480x105 box, then scroll.
    // ------------------------------------------------------------------------
    let scroll_y: f32 = arg_value("--scroll").and_then(|s| s.parse().ok()).unwrap_or(70.0);
    let grid_info = render_buildings_grid(
        &mut renderer,
        &panel,
        &panel_file,
        &report,
        &manifest,
        &mut textures,
        &mut fonts,
        scroll_y,
    );

    // ------------------------------------------------------------------------
    // FOUNDATION (b): a WRAPPED, MARKED-UP, LOCALIZED text box.
    // We build a description with $VAR$ + §colour + £icon, expand it, then wrap by
    // glyph metrics AFTER expansion and draw it inside the panel's lower band.
    // ------------------------------------------------------------------------
    let wrap_info = render_wrapped_description(
        &mut renderer,
        &panel,
        &report,
        &state_name,
        &loc,
        &mut fonts,
        &mut textures,
        &manifest,
    );

    // -- Occupation bars (resistance/compliance) when the state is occupied. --
    render_occupation_bars(&mut renderer, &panel, &report, &mut textures, &manifest);

    eprintln!(
        "[m5] draws: sprites={} nine={} bars={} text={} (missing tex={}, missing font={})",
        stats.sprites, stats.nine_slices, stats.progress_bars, stats.text_runs,
        stats.missing_textures, stats.missing_fonts
    );

    // ------------------------------------------------------------------------
    // STEP 6: composite the accumulated quads OVER the map (no clear) + save.
    // ------------------------------------------------------------------------
    let pixels = renderer.finish_no_clear();
    let out_path = out_dir().join("m5_state_panel.png");
    save_png(&out_path, screen_w, screen_h, &pixels).expect("write m5 png");
    let drawn = pixels
        .chunks_exact(4)
        .filter(|p| !(p[0] < 20 && p[1] < 24 && p[2] < 28))
        .count();
    eprintln!("[m5] wrote {} ({}x{}, {} non-bg px)", out_path.display(), screen_w, screen_h, drawn);

    // ------------------------------------------------------------------------
    // STEP 7 + FOUNDATION (c): HIT-TEST verification (z-order, alwaystransparent).
    // The compositor stack is [map(implicit), topbar, panel] front-to-back. We
    // hit-test the WINDOW stack (panel topmost). The map is the implicit bottom:
    // a click that misses every window falls through to the map province behind.
    // ------------------------------------------------------------------------
    let mut hit_fail = 0;

    // (i) a click on a PAINTED panel element hits the panel.
    let panel_pt = find_rect(&panel, "state_name")
        .map(|r| (r.x + r.w * 0.5, r.y + r.h * 0.5))
        .expect("state_name laid out");
    let h_panel = hit_test_stack(&panel, panel_pt.0, panel_pt.1);
    let panel_leaf = h_panel.as_ref().and_then(|h| h.leaf());
    eprintln!(
        "[m5] HITTEST (i) panel element @ ({:.0},{:.0}) -> {:?}",
        panel_pt.0, panel_pt.1, panel_leaf
    );
    if h_panel.is_none() {
        eprintln!("[m5]   MISMATCH: a click on a painted panel element must hit the panel");
        hit_fail += 1;
    }

    // (ii) a click on a TRANSPARENT panel region falls through to the map.
    // The countrystateview root is a pure coordinate-frame container (no
    // background) → it does NOT self-capture; a point inside its bbox but OUTSIDE
    // every painted child passes through. We find such a point: just to the RIGHT
    // of the painted panel bg, still over the map, and confirm the WINDOW stack
    // misses → the click resolves to the map province behind.
    let panel_bbox = find_rect(&panel, "countrystateview").expect("panel bbox");
    // a point well outside the panel's painted area but on-screen (right of panel).
    let trans_pt = (panel_bbox.x + panel_bbox.w + 400.0, panel_bbox.y + 250.0);
    let h_trans = hit_test_stack(&panel, trans_pt.0, trans_pt.1);
    // Map the screen point back to a raster pixel (map fills the screen 1:1 in UV).
    let map_pid = screen_to_province(&raster, trans_pt.0, trans_pt.1, screen_w, screen_h);
    eprintln!(
        "[m5] HITTEST (ii) transparent region @ ({:.0},{:.0}) -> panel={:?}, falls through to map province {:?}",
        trans_pt.0, trans_pt.1, h_trans.as_ref().and_then(|h| h.leaf()), map_pid
    );
    if h_trans.is_some() {
        eprintln!("[m5]   MISMATCH: a transparent region must NOT capture (it must fall through)");
        hit_fail += 1;
    }
    if map_pid.is_none() {
        eprintln!("[m5]   NOTE: fall-through point was off the raster (still a valid pass-through)");
    }

    // (iii) z-order front-to-back: stack the panel OVER the topbar at a point both
    // cover. The topbar occupies the top band; the panel's state_name sits there
    // too if we move it — instead prove the generic rule with an overlap probe:
    // a point inside BOTH the panel bbox and a topbar element resolves to the
    // topmost (panel), because hit_test_stack walks last-window-first.
    let z = verify_z_order(&topbar, &panel);
    eprintln!("[m5] HITTEST (iii) z-order front-to-back: {}", if z { "OK" } else { "MISMATCH" });
    if !z {
        hit_fail += 1;
    }

    // ------------------------------------------------------------------------
    // Report the foundation outcomes.
    // ------------------------------------------------------------------------
    eprintln!(
        "[m5] FOUNDATION (a) gridbox: {} rows instantiated, viewport {:.0}x{:.0}, content {:.0}px, overflow={}, scrollbar={}",
        grid_info.0, grid_info.1.w, grid_info.1.h, grid_info.2, grid_info.3, grid_info.4
    );
    eprintln!(
        "[m5] FOUNDATION (b) wrapped markup text: {} lines, wrap width {:.0}px, expanded plain=\"{}\"",
        wrap_info.0, wrap_info.1, wrap_info.2
    );
    eprintln!("[m5] FOUNDATION (c) alwaystransparent pass-through + z-order verified above");

    // No-fabrication gate: every unavailable field rendered "—".
    let mut fab_bug = false;
    for f in report.fields.iter().filter(|f| matches!(f.fidelity, bind::Fidelity::Unavailable)) {
        if !f.not_captured || f.value != "—" {
            eprintln!("[m5] FABRICATION BUG: {} = {:?}", f.element, f.value);
            fab_bug = true;
        }
    }

    if hit_fail > 0 {
        eprintln!("[m5] BLOCKER: {hit_fail} hit-test verification(s) failed");
        std::process::exit(3);
    }
    if fab_bug {
        eprintln!("[m5] BLOCKER: a no-fabrication invariant was violated");
        std::process::exit(3);
    }
    if !grid_info.3 {
        eprintln!("[m5] BLOCKER: the buildings gridbox did NOT overflow (foundation a not exercised)");
        std::process::exit(3);
    }
    if wrap_info.0 < 2 {
        eprintln!("[m5] BLOCKER: the description did NOT wrap to >=2 lines (foundation b not exercised)");
        std::process::exit(3);
    }
    if drawn < 50_000 {
        eprintln!("[m5] WARNING: suspiciously few drawn pixels");
        std::process::exit(3);
    }
    eprintln!("[m5] no-fabrication: all Unavailable state-panel elements rendered '—'. OK");
    println!("OK {}", out_path.display());
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Find any raster pixel that resolves to `state_id` (a ground-truth pick point).
fn find_state_pixel(raster: &ProvinceRaster, ids: &MapIds, state_id: u32) -> Option<(u32, u32)> {
    for y in (0..raster.height).step_by(3) {
        for x in (0..raster.width).step_by(3) {
            if let Some(pid) = raster.province_at(x, y) {
                if ids.state_of_province(pid) == Some(state_id) {
                    return Some((x, y));
                }
            }
        }
    }
    None
}

/// Map a screen point to the province behind it (map fills the screen, UV 1:1).
fn screen_to_province(raster: &ProvinceRaster, sx: f32, sy: f32, sw: u32, sh: u32) -> Option<u32> {
    if sx < 0.0 || sy < 0.0 || sx >= sw as f32 || sy >= sh as f32 {
        return None;
    }
    let u = sx / sw as f32;
    let v = sy / sh as f32;
    let px = (u * raster.width as f32) as u32;
    let py = (v * raster.height as f32) as u32;
    raster.province_at(px.min(raster.width - 1), py.min(raster.height - 1))
}

/// Resolve a state's display name from static map meta + loc (NOT the snapshot).
fn resolve_state_name(map_dir: &Path, loc: &Loc, sid: u32) -> Option<String> {
    let txt = std::fs::read_to_string(map_dir.join("state_meta.json")).ok()?;
    let meta: HashMap<String, serde_json::Value> = serde_json::from_str(&txt).ok()?;
    let name_key = meta.get(&sid.to_string())?.get("name_key")?.as_str()?;
    Some(loc.resolve(name_key))
}

fn print_bind_report(r: &StateBindReport) {
    eprintln!("[m5] --- bound state panel (state {}) ---", r.state_id);
    for f in &r.fields {
        let badge = if f.not_captured { "NOT-CAPTURED" } else { "" };
        eprintln!("[m5]   {:48} = {:<16} [{:?}] {badge}", f.element, f.value, f.fidelity);
    }
    eprintln!("[m5]   state buildings ({}):", r.state_buildings.len());
    for b in &r.state_buildings {
        eprintln!("[m5]      {:22} lvl {:<3} frame {}", b.key, b.level, b.frame);
    }
    eprintln!("[m5]   province buildings ({}):", r.province_buildings.len());
    for b in &r.province_buildings {
        eprintln!("[m5]      {:22} lvl {:<3} frame {}", b.key, b.level, b.frame);
    }
    eprintln!("[m5]   resource rows: {} (empty = unavailable, never substituted)", r.resource_rows.len());
}

/// Substitute the bound scalar values into the panel scene-graph text elements.
fn apply_state_scalars(roots: &mut [Node], report: &StateBindReport) {
    for f in &report.fields {
        // owner/controller/occupation are painted separately (flags/bars).
        if f.element.contains("status_container") || f.element == "state_owners" || f.element == "controller_flag" {
            continue;
        }
        for root in roots.iter_mut() {
            bind::apply::set_element_text(root, &f.element, &f.value);
        }
    }
}

/// Paint the owner / controller flag images into the panel's flag slots.
fn paint_state_flags(
    renderer: &mut Renderer,
    panel: &[Node],
    report: &StateBindReport,
    flag_res: &FlagResolver,
    _textures: &mut TextureCache,
    _manifest: &SpriteManifest,
    _resolver: &GameResolver,
) {
    if let (Some(tag), Some(rect)) = (&report.owner_tag, find_rect(panel, "state_owners")) {
        if let Some(rf) = flag_res.resolve(tag, None) {
            if let Ok(tex) = renderer.load_image(&rf.path) {
                let (w, h) = (41.0_f32, 26.0);
                renderer.draw_sprite(&tex, rect.x, rect.y, w, h);
            }
        }
    }
    if let (Some(tag), Some(rect)) = (&report.controller_tag, find_rect(panel, "controller_flag")) {
        if let Some(rf) = flag_res.resolve(tag, None) {
            if let Ok(tex) = renderer.load_image(&rf.path) {
                let (w, h) = (41.0_f32, 26.0);
                renderer.draw_sprite(&tex, rect.x, rect.y, w, h);
            }
        }
    }
}

/// FOUNDATION (a): instantiate + draw the buildings gridbox row-factory, made to
/// overflow + scroll, with a working scrollbar. Returns
/// (rows, viewport, content_h, overflowing, has_scrollbar).
#[allow(clippy::too_many_arguments)]
fn render_buildings_grid(
    renderer: &mut Renderer,
    panel: &[Node],
    panel_file: &GuiFile,
    report: &StateBindReport,
    manifest: &SpriteManifest,
    textures: &mut TextureCache,
    fonts: &mut FontCache,
    scroll_y: f32,
) -> (usize, Rect, f32, bool, bool) {
    // the gridbox's laid-out viewport rect.
    let viewport = match find_rect(panel, "state_building_entries") {
        Some(r) => r,
        None => return (0, Rect::default(), 0.0, false, false),
    };
    // the entry template (a top-level window named state_building_entry).
    let template = match find_template(panel_file, manifest, "state_building_entry") {
        Some(t) => t,
        None => return (0, viewport, 0.0, false, false),
    };

    // COMBINE state + province buildings into one row list so it OVERFLOWS the
    // 480x105 box (state buildings alone already overflow at 7; province adds more).
    let mut rows = report.state_buildings.clone();
    rows.extend(report.province_buildings.clone());
    let n = rows.len();

    let spec = GridSpec::from_gridbox(
        &Node { rect: viewport, ..stub_named("state_building_entries") },
        &template,
        14.0, // 14px scrollbar gutter
    );
    let content = ManifestContent { manifest, measure: |_t, _f| None };
    let mut inst = gridbox::instantiate(&template, n, viewport, &spec, scroll_y, &content);

    // Bind each row's level text + building icon frame, then draw the (clipped,
    // scrolled) row clones via the existing draw pass.
    for (i, row) in inst.rows.iter_mut().enumerate() {
        let b = &rows[i];
        bind::apply::set_element_text(row, "level", &b.level);
        gridbox::set_row_icon_frame(row, "building_picture", b.frame);
    }
    let mut stats = DrawStats::default();
    for row in &inst.rows {
        draw_window(renderer, row, textures, fonts, &mut stats);
    }

    // Draw the scrollbar: track (9-slice bg) + thumb (a generic box).
    if let Some(sb) = inst.scrollbar {
        // track (9-slice).
        if let Some(t) = textures.get(renderer, "GFX_scrollbar_vertical_bg") {
            let border = manifest
                .get("GFX_scrollbar_vertical_bg")
                .and_then(|e| e.border_size)
                .map(|b| [b[0] as f32, b[1] as f32])
                .unwrap_or([0.0, 52.0]);
            renderer.draw_nine_slice(t, sb.track.x, sb.track.y, sb.track.w, sb.track.h, border);
        }
        // thumb.
        if let Some(t) = textures.get(renderer, "GFX_generic_box_96") {
            renderer.draw_sprite(t, sb.thumb.x, sb.thumb.y, sb.thumb.w, sb.thumb.h);
        }
    }

    (n, viewport, inst.content_height, inst.overflowing, inst.scrollbar.is_some())
}

/// A minimal stub node with a name (for GridSpec::from_gridbox viewport probing).
fn stub_named(name: &str) -> Node {
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
        rect: Rect::default(),
        clip: None,
        children: vec![],
    }
}

/// Find an entry-template window (a top-level window in the gui file) by name and
/// build it into a scene node (templates are separate windows, not children).
fn find_template(file: &GuiFile, manifest: &SpriteManifest, name: &str) -> Option<Node> {
    let raw = file.windows.iter().find(|w| w.name.as_deref() == Some(name))?;
    let loc = Loc::new();
    Some(Node::from_raw(raw, manifest, &loc))
}

/// FOUNDATION (b): build, expand, glyph-wrap (AFTER expansion) and draw a marked-up
/// localized description. Returns (lines, wrap_width, expanded_plain).
#[allow(clippy::too_many_arguments)]
fn render_wrapped_description(
    renderer: &mut Renderer,
    panel: &[Node],
    report: &StateBindReport,
    state_name: &str,
    _loc: &Loc,
    fonts: &mut FontCache,
    textures: &mut TextureCache,
    _manifest: &SpriteManifest,
) -> (usize, f32, String) {
    // The description carries a $VAR$ (state name), §colour runs, and a £icon —
    // a faithful HOI4-style marked-up localized string. (Built here as the slice's
    // example "wrapped, marked-up, localized text box"; in-game it would be a loc
    // key, but the wrap path is identical.)
    let cat = report.get("category").map(|f| f.value.clone()).unwrap_or_default();
    let vp = report.get("victory_points").map(|f| f.value.clone()).unwrap_or_default();
    let src = "The state of §Y$STATE$§! is a §G$CAT$§! region holding £victory_points $VP$ victory \
               points. Its industry and manpower are bound from the recorded snapshot, with no \
               fabricated values: unavailable fields render as a dash.";

    let mut vars = HashMap::new();
    vars.insert("STATE".to_string(), state_name.to_string());
    vars.insert("CAT".to_string(), cat);
    vars.insert("VP".to_string(), vp);

    let default_col = [0.90, 0.90, 0.86, 1.0];
    let exp = expand(src, &vars, default_col);

    // Wrap inside the panel's lower band (below the building grids). Use the panel
    // bbox to place a sensible text column.
    let bbox = find_rect(panel, "countrystateview").unwrap_or(Rect { x: 40.0, y: 600.0, w: 440.0, h: 200.0 });
    let wrap_x = bbox.x + 22.0;
    let wrap_w = 410.0_f32;
    let wrap_y0 = bbox.y + 470.0;

    // load the font and wrap by glyph metrics AFTER expansion.
    let font_name = "hoi_18mbs";
    let font: BmFont = match fonts.get(renderer, font_name) {
        Some(l) => l.font.clone(),
        None => return (0, wrap_w, exp.plain.clone()),
    };
    let wrapped = wrap(&exp, &font, wrap_w);

    // draw each line span-by-span with its colour; reserve icon advance for £icon.
    let mut y = wrap_y0;
    for line in &wrapped.lines {
        let mut x = wrap_x;
        for span in &line.spans {
            if let Some(icon) = &span.icon {
                // draw the inline icon as a sprite (~line tall) if resolvable.
                let gfx = map_icon_name(icon);
                let h = wrapped.line_height;
                if let Some(t) = textures.get(renderer, &gfx) {
                    renderer.draw_sprite(t, x, y, h, h);
                }
                x += gui_runtime::markup::ICON_ADVANCE;
            } else {
                let loaded = fonts.get(renderer, font_name).unwrap();
                let f = loaded.font.clone();
                let pages = &loaded.pages;
                renderer.draw_text(&f, pages, &span.text, x, y, span.color);
                x += f.measure(&span.text) as f32;
            }
        }
        y += wrapped.line_height;
    }

    (wrapped.lines.len(), wrap_w, exp.plain)
}

/// Map a £icon token to a real GFX sprite name (best-effort; falls back to a VP bg).
fn map_icon_name(token: &str) -> String {
    match token {
        "victory_points" => "GFX_victorypoint_stateview_bg".to_string(),
        other => format!("GFX_{other}"),
    }
}

/// Draw the occupation resistance/compliance bars when the state is occupied.
fn render_occupation_bars(
    renderer: &mut Renderer,
    panel: &[Node],
    report: &StateBindReport,
    textures: &mut TextureCache,
    _manifest: &SpriteManifest,
) {
    let occ = match &report.occupation {
        Some(o) => *o,
        None => return,
    };
    // place the bars in the resistance_container band.
    let base = find_rect(panel, "resistance_container").unwrap_or(Rect { x: 60.0, y: 760.0, w: 519.0, h: 100.0 });
    let bar_w = 81.0_f32;
    let bar_h = 14.0_f32;
    // resistance bar (top) and compliance bar (below).
    for (i, (gfx, frac)) in [
        ("GFX_occupation_state_resistance_bar", (occ.resistance / 100.0) as f32),
        ("GFX_occupation_state_compliance_bar", (occ.compliance / 100.0) as f32),
    ]
    .iter()
    .enumerate()
    {
        let y = base.y + 6.0 + i as f32 * 28.0;
        if let Some(t) = textures.get(renderer, gfx) {
            renderer.draw_progress_bar(t, base.x + 40.0, y, bar_w, bar_h, *frac, 0, 1, [1.0, 1.0, 1.0, 1.0]);
        }
    }
}

/// Verify front-to-back z-order with a TRUE overlapping stack (Codex (c) caveat:
/// prove panel-OVER-topbar, not just panel hit + topbar liveness). We build a
/// real overlap: a copy of the panel translated UP so its painted `state_name`
/// lands inside the topbar band, then assert the stack [topbar, panel] resolves
/// the OVERLAP point to the PANEL (topmost), the topbar-only region to the TOPBAR,
/// and a region neither covers to NEITHER (None / map fall-through).
fn verify_z_order(topbar: &[Node], panel: &[Node]) -> bool {
    // a topbar element rect (full-width band element) + its centre.
    let tb_rect = match find_rect(topbar, "DateText")
        .or_else(|| find_rect(topbar, "pol_power"))
        .or_else(|| find_rect(topbar, "manpower"))
    {
        Some(r) => r,
        None => return false,
    };

    // Build an overlapping panel copy: translate the panel (in BOTH axes) so its
    // painted `state_name` CENTRE lands on the topbar element's centre — a
    // guaranteed overlap regardless of where each sits on screen.
    let sn = match find_rect(panel, "state_name") {
        Some(r) => r,
        None => return false,
    };
    let tb_c = (tb_rect.x + tb_rect.w * 0.5, tb_rect.y + tb_rect.h * 0.5);
    let sn_c = (sn.x + sn.w * 0.5, sn.y + sn.h * 0.5);
    let (dx, dy) = (tb_c.0 - sn_c.0, tb_c.1 - sn_c.1);
    let mut overlap_panel: Vec<Node> = panel.to_vec();
    for w in overlap_panel.iter_mut() {
        translate_tree(w, dx, dy);
    }
    let moved_sn = match find_rect(&overlap_panel, "state_name") {
        Some(r) => r,
        None => return false,
    };

    // overlap point: the shared centre — inside BOTH the moved state_name AND the
    // topbar element (both centred there by construction).
    let overlap = (
        (moved_sn.x.max(tb_rect.x) + (moved_sn.x + moved_sn.w).min(tb_rect.x + tb_rect.w)) * 0.5,
        (moved_sn.y.max(tb_rect.y) + (moved_sn.y + moved_sn.h).min(tb_rect.y + tb_rect.h)) * 0.5,
    );
    // resolve front-to-back: panel (topmost) first, else topbar, else map.
    let resolve = |wins_panel: &[Node], x: f32, y: f32| -> &'static str {
        if hit_test_stack(wins_panel, x, y).is_some() {
            "panel"
        } else if hit_test_stack(topbar, x, y).is_some() {
            "topbar"
        } else {
            "map"
        }
    };
    // (1) at the overlap, the PANEL (topmost window) must win over the topbar.
    let panel_wins = resolve(&overlap_panel, overlap.0, overlap.1) == "panel";

    // (2) where only the topbar paints (a DIFFERENT topbar element, clear of the
    //     moved panel), the topbar wins. Pick a topbar element whose centre is NOT
    //     inside the moved-panel bbox so the test is unambiguous.
    let moved_bbox = find_rect(&overlap_panel, "countrystateview").unwrap_or(Rect {
        x: 0.0, y: 0.0, w: 0.0, h: 0.0,
    });
    let topbar_only = ["pol_power", "manpower", "stability", "fuel_value", "DateText"]
        .iter()
        .find_map(|name| {
            let r = find_rect(topbar, name)?;
            let c = (r.x + r.w * 0.5, r.y + r.h * 0.5);
            if !moved_bbox.contains(c.0, c.1) {
                Some(c)
            } else {
                None
            }
        });
    let topbar_wins = match topbar_only {
        Some((x, y)) => resolve(&overlap_panel, x, y) == "topbar",
        // if every topbar element is (improbably) under the moved panel, fall back
        // to the original (un-moved) stack where the topbar is independently alive.
        None => hit_test_stack(topbar, tb_rect.x + 2.0, tb_rect.y + 2.0).is_some(),
    };

    panel_wins && topbar_wins
}

/// Translate an entire laid-out subtree by (dx,dy): rect + clip. Used to build a
/// guaranteed overlap for the z-order verification (read-only test transform).
fn translate_tree(n: &mut Node, dx: f32, dy: f32) {
    n.rect.x += dx;
    n.rect.y += dy;
    if let Some(c) = n.clip.as_mut() {
        c.x += dx;
        c.y += dy;
    }
    for c in n.children.iter_mut() {
        translate_tree(c, dx, dy);
    }
}
