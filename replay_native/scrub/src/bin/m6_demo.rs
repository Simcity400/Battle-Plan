//! m6_demo — THE SCRUBBER, end to end against real data.
//!
//! Proves the M6 spine: a global `hour_index` drives an IN-PROCESS re-bind of
//! every open surface (the M3 topbar + the M4 map + the M5 state panel) over a
//! snapshot SERIES read from the store index. The series is walked, never
//! reconstructed. Each cursor move is served through a WINDOWED IMMUTABLE LRU of
//! parsed snapshots: a warm hit is an `Arc::clone` (≈ 0 ms) vs a ~100 ms cold parse.
//!
//! Pipeline (all in-process, offscreen, READ-ONLY on assets + the store):
//!   1. Open a `Timeline` over a MULTI-snapshot game (the `campaign` game = 2
//!      snapshots: hours 17000461 + 17050260) — series straight from index.json.
//!   2. SCRUB to hour A: seek -> Frame -> re-bind topbar + map + panel from A's
//!      domain-scoped view -> compose -> render/out/m6_scrub_hourA.png.
//!   3. SCRUB to hour B: seek -> Frame (cold) -> re-bind ALL surfaces from B ->
//!      compose -> render/out/m6_scrub_hourB.png. Topbar values change, the map
//!      recolours, the panel updates.
//!   4. SCRUB BACK to A: seek -> Frame (WARM, same Arc) -> re-bind -> assert the
//!      re-bind is IDENTICAL to step 2 (the A->B->A no-fabrication round trip).
//!   5. Report cold-vs-warm re-bind latency; confirm the warm hit ≈ 0 ms.
//!
//! Paths via CLI/env/documented defaults — no personal literal in committed source.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use assets::resolver::GameResolver;
use assets::{BmFont, SpriteManifest};
use bind::{apply_topbar, bind_state, BindReport, FlagResolver, StateBindReport, TopbarSpec};
use gui_runtime::draw::{draw_window, DrawStats, TextureCache};
use gui_runtime::fonts::{FontCache, FontMap};
use gui_runtime::gridbox::{self, GridSpec};
use gui_runtime::scene::{ElementKind, Node};
use gui_runtime::{build_scene, expand, layout, wrap, GuiFile, Loc, ManifestContent, Rect};
use render::map::{
    build_lut, CountryColors, MapIds, PoliticalMap, ProvinceRaster, StateOwners,
};
use render::{save_png, Renderer};
use scrub::{Fetch, Frame, Timeline};
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

/// Repo root = two levels up from this crate (replay_native/scrub -> repo).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The committed output dir is render/out/ (sibling crate), matching M1–M5.
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

// ===========================================================================
// Static (hour-independent) assets, loaded ONCE and reused across every scrub
// tick — only the snapshot + the per-tick re-bind changes between hours.
// ===========================================================================
struct Stage {
    resolver: GameResolver,
    map_dir: PathBuf,
    manifest: SpriteManifest,
    loc: Loc,
    topbar_file: GuiFile,
    panel_file: GuiFile,
    raster: ProvinceRaster,
    ids: MapIds,
    colors: CountryColors,
    screen_w: u32,
    screen_h: u32,
}

fn main() {
    let country = arg_value("--country").unwrap_or_else(|| "GER".to_string());
    // DEFAULT = the campaign game (2 snapshots), the spec's named multi-snapshot game.
    let game = arg_value("--game").unwrap_or_else(|| "campaign".to_string());
    // DEFAULT state = 6 (a Belgian state present in BOTH snapshots whose panel data
    // visibly RE-BINDS across the scrub: at A it is GER-occupied (controller GER,
    // occupation bars shown); at B it is liberated (controller BEL, no occupation).
    // This makes the panel re-bind observable. `--state 64` picks a static state.
    let target_state: u32 = arg_value("--state").and_then(|s| s.parse().ok()).unwrap_or(6);

    let vanilla = arg_value("--vanilla");
    let mod_root = arg_value("--mod");
    let resolver = GameResolver::new(vanilla.as_deref(), mod_root.as_deref());

    let map_dir = arg_value("--map")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("map_pipeline").join("assets"));
    let store_root = arg_value("--store")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("replay").join("snapshots"));

    eprintln!("[m6] country={country} game={game} state={target_state}");
    eprintln!("[m6] vanilla = {}", resolver.vanilla.display());
    if !resolver.vanilla_exists() {
        eprintln!("[m6] BLOCKER: game install not found. Pass --vanilla <path> or set BATTLE_PLAN_VANILLA.");
        std::process::exit(2);
    }
    let assets_dir = find_assets_dir().unwrap_or_else(|| {
        eprintln!("[m6] BLOCKER: game_assets/gui/province_state.json not found (pass --assets).");
        std::process::exit(2);
    });
    eprintln!("[m6] assets = {}", assets_dir.display());
    eprintln!("[m6] map    = {}", map_dir.display());
    eprintln!("[m6] store  = {} (game {game})", store_root.display());

    // ----------------------------------------------------------------------
    // THE SCRUBBER: open the timeline over the store index (the SERIES). Never
    // reconstructed — read verbatim from index.json, ascending.
    // ----------------------------------------------------------------------
    let store = Store::open(&store_root);
    let mut timeline = Timeline::open(store, game.clone(), 6).unwrap_or_else(|e| {
        eprintln!("[m6] BLOCKER: open timeline for game {game}: {e}");
        std::process::exit(2);
    });
    if timeline.len() < 2 {
        eprintln!(
            "[m6] BLOCKER: game {game} has {} snapshot(s); M6 needs a MULTI-snapshot game. \
             Try --game campaign (2 snapshots).",
            timeline.len()
        );
        std::process::exit(2);
    }
    eprintln!(
        "[m6] timeline: {} snapshots in series: {:?}",
        timeline.len(),
        timeline.hours()
    );

    // Load the hour-independent stage ONCE.
    let stage = load_stage(&resolver, &assets_dir, &map_dir);

    // Pick the series endpoints A (first) and B (last).
    let pos_a = 0usize;
    let pos_b = timeline.len() - 1;
    let hour_a = timeline.hours()[pos_a];
    let hour_b = timeline.hours()[pos_b];
    eprintln!("[m6] A = pos {pos_a} hour {hour_a}; B = pos {pos_b} hour {hour_b}");

    // ----------------------------------------------------------------------
    // STEP 2: scrub to A (cold) -> re-bind all surfaces -> compose -> PNG.
    // We TIME the fetch (cold parse) and the re-bind separately.
    // ----------------------------------------------------------------------
    let t = Instant::now();
    let frame_a = timeline.seek_to(pos_a).expect("seek A");
    let fetch_a_ms = t.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(frame_a.fetch, Fetch::Cold, "first A touch must be a cold parse");
    eprintln!(
        "[m6] SCRUB -> A hour {} [{:?}] fetch {:.1} ms",
        frame_a.hour_index, frame_a.fetch, fetch_a_ms
    );

    let (rebind_a_cold_ms, report_a1) = rebind_and_compose(
        &stage,
        &frame_a,
        &country,
        target_state,
        &out_dir().join("m6_scrub_hourA.png"),
        "A",
    );

    // ----------------------------------------------------------------------
    // STEP 3: scrub to B (cold) -> re-bind ALL surfaces -> compose -> PNG.
    // ----------------------------------------------------------------------
    let t = Instant::now();
    let frame_b = timeline.seek_to(pos_b).expect("seek B");
    let fetch_b_ms = t.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(frame_b.fetch, Fetch::Cold, "first B touch must be a cold parse");
    eprintln!(
        "[m6] SCRUB -> B hour {} [{:?}] fetch {:.1} ms",
        frame_b.hour_index, frame_b.fetch, fetch_b_ms
    );

    let (_rebind_b_ms, report_b) = rebind_and_compose(
        &stage,
        &frame_b,
        &country,
        target_state,
        &out_dir().join("m6_scrub_hourB.png"),
        "B",
    );

    // ----------------------------------------------------------------------
    // STEP 4: scrub BACK to A -> WARM hit (same Arc). Re-bind + assert the A
    // re-bind is IDENTICAL to step 2 — the no-fabrication round trip.
    // ----------------------------------------------------------------------
    let t = Instant::now();
    let frame_a2 = timeline.seek_to(pos_a).expect("seek A again");
    let fetch_a_warm_ms = t.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(frame_a2.fetch, Fetch::Warm, "returning to A must be a cache HIT");
    assert!(
        std::sync::Arc::ptr_eq(&frame_a.snapshot, &frame_a2.snapshot),
        "the A revisit must be the SAME shared parse"
    );
    eprintln!(
        "[m6] SCRUB -> A (back) hour {} [{:?}] fetch {:.3} ms (cache hit)",
        frame_a2.hour_index, frame_a2.fetch, fetch_a_warm_ms
    );

    // Re-bind the warm A and time it (this is the warm-scrub re-bind cost). We do a
    // FULL re-compose to a PNG (not just rebind_only) so the round trip is proven at
    // the PIXEL level too (Codex (c) caveat: A2 must re-render, not only re-report).
    let a2_png = out_dir().join("m6_scrub_hourA_revisit.png");
    let t = Instant::now();
    let (rebind_a_warm_ms_full, report_a2) = rebind_and_compose(
        &stage,
        &frame_a2,
        &country,
        target_state,
        &a2_png,
        "A'",
    );
    let _ = t; // (rebind time is reported by rebind_and_compose's internal timer)
    let rebind_a_warm_ms = rebind_a_warm_ms_full;

    // THE prime-directive assertion at the end-to-end level: the A digest after the
    // B visit must EQUAL the A digest before it. No carried-over / defaulted /
    // fabricated value from B.
    let digest_a1 = digest(&report_a1.0, &report_a1.1);
    let digest_a2 = digest(&report_a2.0, &report_a2.1);
    let report_identical = digest_a1 == digest_a2;

    // PIXEL-level proof: the re-rendered A (after B) must be byte-identical to the
    // first A render. Identical report + identical static assets => identical pixels.
    let a1_png = out_dir().join("m6_scrub_hourA.png");
    let pixels_identical = match (std::fs::read(&a1_png), std::fs::read(&a2_png)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    let identical = report_identical && pixels_identical;
    eprintln!(
        "[m6] A->B->A identity: report={} pixels={} -> {}",
        report_identical,
        pixels_identical,
        if identical { "IDENTICAL (no fabrication across the round trip)" } else { "MISMATCH" }
    );
    // the revisit PNG is a throwaway proof artifact; remove it so only A + B remain.
    let _ = std::fs::remove_file(&a2_png);

    // ----------------------------------------------------------------------
    // STEP 5: the latency report + no-fabrication audit + verdicts.
    // ----------------------------------------------------------------------
    let c = timeline.cache();
    eprintln!("[m6] ============ LATENCY ============");
    eprintln!("[m6]   COLD fetch A (read+parse)   : {:.1} ms", fetch_a_ms);
    eprintln!("[m6]   COLD fetch B (read+parse)   : {:.1} ms", fetch_b_ms);
    eprintln!("[m6]   WARM fetch A (cache hit)     : {:.3} ms  <- ~0", fetch_a_warm_ms);
    eprintln!("[m6]   COLD re-bind A (all surfaces): {:.2} ms", rebind_a_cold_ms);
    eprintln!("[m6]   WARM re-bind A (all surfaces): {:.2} ms (pure bind, no fetch/parse)", rebind_a_warm_ms);
    eprintln!(
        "[m6]   LRU: window={} resident={:?} hits={} misses={} evictions={}",
        c.capacity(), c.resident_hours(), c.hits, c.misses, c.evictions
    );
    let speedup = if fetch_a_warm_ms > 0.0 { fetch_a_ms / fetch_a_warm_ms } else { f64::INFINITY };
    eprintln!("[m6]   warm/cold fetch speedup      : {:.0}x", speedup);

    // No-fabrication: every Unavailable field in BOTH the topbar and panel reports
    // (at A and B) must render "—", never a fabricated value.
    let mut fab_bug = false;
    for (label, (tb, sp)) in [("A", &report_a2), ("B", &report_b)] {
        for f in tb.fields.iter().filter(|f| matches!(f.fidelity, bind::Fidelity::Unavailable)) {
            if !f.not_captured || f.value != "—" {
                eprintln!("[m6] FABRICATION BUG @ {label} topbar: {} = {:?}", f.element, f.value);
                fab_bug = true;
            }
        }
        for f in sp.fields.iter().filter(|f| matches!(f.fidelity, bind::Fidelity::Unavailable)) {
            if !f.not_captured || f.value != "—" {
                eprintln!("[m6] FABRICATION BUG @ {label} panel: {} = {:?}", f.element, f.value);
                fab_bug = true;
            }
        }
    }

    // Prove the surfaces actually RE-BOUND between A and B (the scrub did something).
    let tb_changed = surfaces_differ_topbar(&report_a2.0, &report_b.0);
    let panel_changed = surfaces_differ_panel(&report_a2.1, &report_b.1);
    eprintln!(
        "[m6] re-bind delta A vs B: topbar changed={} panel changed={}",
        tb_changed, panel_changed
    );

    // -- final gates --
    if !identical {
        eprintln!("[m6] BLOCKER: the A->B->A round trip was NOT identical (no-fabrication violated)");
        std::process::exit(3);
    }
    if fab_bug {
        eprintln!("[m6] BLOCKER: a no-fabrication invariant was violated");
        std::process::exit(3);
    }
    if !tb_changed && !panel_changed {
        eprintln!("[m6] BLOCKER: no surface re-bound between A and B (the scrub did nothing)");
        std::process::exit(3);
    }
    // warm fetch must be dramatically cheaper than cold (the LRU's whole point).
    if fetch_a_warm_ms >= fetch_a_ms.max(1.0) {
        eprintln!("[m6] BLOCKER: a warm cache hit was not cheaper than the cold parse");
        std::process::exit(3);
    }

    eprintln!("[m6] no-fabrication: all Unavailable surface fields rendered '—' at A and B. OK");
    println!(
        "OK A={} B={} (cold {:.0}ms, warm {:.3}ms, {:.0}x)",
        out_dir().join("m6_scrub_hourA.png").display(),
        out_dir().join("m6_scrub_hourB.png").display(),
        fetch_a_ms,
        fetch_a_warm_ms,
        speedup
    );
}

// ===========================================================================
// Stage loading (once).
// ===========================================================================
fn load_stage(resolver: &GameResolver, assets_dir: &Path, map_dir: &Path) -> Stage {
    let manifest =
        SpriteManifest::load(&assets_dir.join("sprites.json")).expect("BLOCKER: load sprites.json");
    let mut loc = Loc::new();
    for base in loc_dirs(resolver) {
        let _ = loc.add_dir(&base);
    }
    let topbar_file = GuiFile::load(&assets_dir.join("gui").join("topbar.json"))
        .expect("BLOCKER: parse topbar.json");
    let panel_file = GuiFile::load(&assets_dir.join("gui").join("province_state.json"))
        .expect("BLOCKER: parse province_state.json");
    let raster = ProvinceRaster::load_png(&map_dir.join("province_ids.png"))
        .expect("BLOCKER: load province_ids.png");
    let ids = MapIds::load(map_dir).expect("BLOCKER: MapIds::load");
    let colors = CountryColors::load(map_dir).expect("BLOCKER: CountryColors::load");
    eprintln!(
        "[m6] stage: loc keys={} sprites={} raster={}x{}",
        loc.len(),
        manifest.sprites.len(),
        raster.width,
        raster.height
    );
    Stage {
        // GameResolver is cheap to rebuild; clone via new() inputs would be awkward,
        // so we move a fresh resolver in (paths are the same).
        resolver: GameResolver::new(
            Some(resolver.vanilla.to_str().unwrap_or("")),
            resolver.mod_root.as_deref().and_then(|p| p.to_str()),
        ),
        map_dir: map_dir.to_path_buf(),
        manifest,
        loc,
        topbar_file,
        panel_file,
        raster,
        ids,
        colors,
        screen_w: 1920,
        screen_h: 1080,
    }
}

// ===========================================================================
// THE RE-BIND: bind every surface from a frame's DOMAIN-SCOPED view. This is the
// in-process spine — moving the scrubber to a new hour calls this against the new
// snapshot, and every surface's bound state is recomputed from that hour alone.
// Returns the topbar + state-panel reports (the no-fabrication outputs).
// ===========================================================================
fn rebind_only(
    stage: &Stage,
    frame: &Frame,
    country: &str,
    target_state: u32,
) -> (BindReport, StateBindReport) {
    let dv = frame.view();

    // --- TOPBAR: bind from the COUNTRY-scoped view (countries.<TAG> only). ---
    // apply_topbar takes &Snapshot but only reads the one country domain; we drive
    // it through the domain view's read-only snapshot handle so the scoping is
    // explicit and uniform with the other surfaces.
    let spec = TopbarSpec::topbar();
    let mut throwaway: Vec<Node> = Vec::new(); // report-only; the composed pass rebuilds its own scene
    let tb_report = apply_topbar(&mut throwaway, &spec, dv.snapshot(), country);

    // --- STATE PANEL: bind from the STATE-scoped view (states.<sid> only). ---
    let state_name = resolve_state_name(&stage.map_dir, &stage.loc, target_state)
        .unwrap_or_else(|| format!("STATE_{target_state}"));
    let sp_report = bind_state(dv.snapshot(), &target_state.to_string(), &state_name);

    (tb_report, sp_report)
}

/// Full re-bind + compose + render to a PNG. Returns (rebind_ms, (topbar, panel)).
fn rebind_and_compose(
    stage: &Stage,
    frame: &Frame,
    country: &str,
    target_state: u32,
    out_path: &Path,
    label: &str,
) -> (f64, (BindReport, StateBindReport)) {
    let (screen_w, screen_h) = (stage.screen_w, stage.screen_h);
    let screen = Rect { x: 0.0, y: 0.0, w: screen_w as f32, h: screen_h as f32 };
    let dv = frame.view();

    // Time the pure re-bind (no GPU) — this is the per-tick scrub cost.
    let t = Instant::now();
    let (tb_report, sp_report) = rebind_only(stage, frame, country, target_state);
    let rebind_ms = t.elapsed().as_secs_f64() * 1000.0;

    // --- MAP: build the political LUT from the MAP-scoped view (states+map). ---
    let map_view = dv.map();
    let states_obj = map_view.states().expect("BLOCKER: snapshot has no states");
    let owners = StateOwners::from_states(states_obj);

    // --- renderer + caches (fresh per compose to keep passes isolated). ---
    let mut renderer = Renderer::new(screen_w, screen_h).expect("BLOCKER: renderer init");
    let font_map = FontMap::from_core_gfx(&stage.resolver);
    let mut fonts = FontCache::new(&font_map, &stage.resolver);
    let mut textures = TextureCache::new(&stage.manifest, &stage.resolver);

    // PASS 1 (bottom): the political map fills the screen, recoloured for THIS hour.
    let lut = build_lut(stage.raster.max_id, &stage.ids, &owners, &stage.colors);
    let pmap = PoliticalMap::new(&renderer, &stage.raster);
    pmap.update_lut(&renderer, &lut);
    let _ = pmap.render(
        &renderer,
        [0.0, 0.0, screen_w as f32, screen_h as f32],
        [0.0, 0.0],
        [1.0, 1.0],
        [0.05, 0.06, 0.08, 1.0],
    );

    // PASS 2 (top): the topbar, re-bound to the observed country at THIS hour.
    let mut topbar = build_scene(&stage.topbar_file, &stage.manifest, &stage.loc);
    let spec = TopbarSpec::topbar();
    let tb_report2 = apply_topbar(&mut topbar, &spec, dv.snapshot(), country);
    let content = ManifestContent { manifest: &stage.manifest, measure: |_t, _f| None };
    for root in topbar.iter_mut() {
        layout(root, screen, None, &content);
    }
    let flag_res = FlagResolver::new(&stage.resolver);
    let tb_flag = flag_res
        .resolve(country, tb_report2.flag_ideology.as_deref())
        .and_then(|rf| {
            let rect = find_rect(&topbar, "player_flag")?;
            renderer.load_image(&rf.path).ok().map(|t| (rect, t))
        });

    // PASS 3 (stacked): the state panel, re-bound to the selected state at THIS hour.
    const PANEL_X: f32 = 40.0;
    const PANEL_Y: f32 = 110.0;
    let mut panel: Vec<Node> = build_scene(&stage.panel_file, &stage.manifest, &stage.loc)
        .into_iter()
        .filter(|w| w.name == "countrystateview")
        .collect();
    for root in panel.iter_mut() {
        root.orientation = gui_runtime::Anchor::UpperLeft;
        root.origo = gui_runtime::Anchor::UpperLeft;
        root.pos = (PANEL_X, PANEL_Y);
    }
    apply_state_scalars(&mut panel, &sp_report);
    for root in panel.iter_mut() {
        layout(root, screen, None, &content);
    }

    // Draw order: map (in target) -> topbar quads -> panel quads.
    let mut stats = DrawStats::default();
    for root in &topbar {
        draw_window(&mut renderer, root, &mut textures, &mut fonts, &mut stats);
    }
    if let Some((r, tex)) = &tb_flag {
        renderer.draw_sprite(tex, r.x, r.y, r.w, r.h);
    }
    for root in &panel {
        draw_window(&mut renderer, root, &mut textures, &mut fonts, &mut stats);
    }
    paint_state_flags(&mut renderer, &panel, &sp_report, &flag_res);

    // Buildings gridbox (overflow + scroll), wrapped markup, occupation bars — the
    // same M5 foundations, re-instantiated for THIS hour's bound rows.
    render_buildings_grid(
        &mut renderer,
        &panel,
        &stage.panel_file,
        &sp_report,
        &stage.manifest,
        &mut textures,
        &mut fonts,
        70.0,
    );
    let state_name = sp_report
        .get("state_name")
        .map(|f| f.value.clone())
        .unwrap_or_default();
    render_wrapped_description(
        &mut renderer,
        &panel,
        &sp_report,
        &state_name,
        &mut fonts,
        &mut textures,
        frame.hour_index,
        frame.view().date().unwrap_or("?"),
    );
    render_occupation_bars(&mut renderer, &panel, &sp_report, &mut textures);

    // Composite the accumulated quads OVER the map (no clear) + save.
    let pixels = renderer.finish_no_clear();
    save_png(out_path, screen_w, screen_h, &pixels).expect("write m6 png");
    let drawn = pixels
        .chunks_exact(4)
        .filter(|p| !(p[0] < 20 && p[1] < 24 && p[2] < 28))
        .count();
    eprintln!(
        "[m6] {label}: hour {} date {} -> wrote {} ({} non-bg px, {} text runs)",
        frame.hour_index,
        frame.view().date().unwrap_or("?"),
        out_path.display(),
        drawn,
        stats.text_runs
    );
    if drawn < 50_000 {
        eprintln!("[m6] WARNING: suspiciously few drawn pixels at {label}");
    }

    (rebind_ms, (tb_report, sp_report))
}

// ===========================================================================
// Comparison / digest helpers (the no-fabrication round-trip proof).
// ===========================================================================

/// A stable string digest of a fully bound frame (topbar + panel + building rows
/// + occupation), used to compare A-before vs A-after the B visit.
fn digest(tb: &BindReport, sp: &StateBindReport) -> String {
    let mut s = String::new();
    for f in &tb.fields {
        s.push_str(&format!("TB:{}={}|{}\n", f.element, f.value, f.not_captured));
    }
    s.push_str(&format!("FLAG:{:?}/{:?}\n", tb.flag_tag, tb.flag_ideology));
    for f in &sp.fields {
        s.push_str(&format!("SP:{}={}|{}\n", f.element, f.value, f.not_captured));
    }
    for b in sp.state_buildings.iter().chain(sp.province_buildings.iter()) {
        s.push_str(&format!("BR:{}={}@{}\n", b.key, b.level, b.frame));
    }
    s.push_str(&format!("OWN:{:?} CTRL:{:?} OCC:{}\n", sp.owner_tag, sp.controller_tag, sp.occupation.is_some()));
    s
}

fn surfaces_differ_topbar(a: &BindReport, b: &BindReport) -> bool {
    for fa in &a.fields {
        if let Some(fb) = b.get(&fa.element) {
            if fa.value != fb.value {
                return true;
            }
        }
    }
    false
}

fn surfaces_differ_panel(a: &StateBindReport, b: &StateBindReport) -> bool {
    if a.controller_tag != b.controller_tag || a.owner_tag != b.owner_tag {
        return true;
    }
    if a.occupation.is_some() != b.occupation.is_some() {
        return true;
    }
    for fa in &a.fields {
        if let Some(fb) = b.get(&fa.element) {
            if fa.value != fb.value {
                return true;
            }
        }
    }
    if a.state_buildings.len() != b.state_buildings.len() {
        return true;
    }
    false
}

// ===========================================================================
// Composition helpers (reused from the M5 demo's conventions verbatim).
// ===========================================================================

fn resolve_state_name(map_dir: &Path, loc: &Loc, sid: u32) -> Option<String> {
    let txt = std::fs::read_to_string(map_dir.join("state_meta.json")).ok()?;
    let meta: HashMap<String, serde_json::Value> = serde_json::from_str(&txt).ok()?;
    let name_key = meta.get(&sid.to_string())?.get("name_key")?.as_str()?;
    Some(loc.resolve(name_key))
}

fn apply_state_scalars(roots: &mut [Node], report: &StateBindReport) {
    for f in &report.fields {
        if f.element.contains("status_container")
            || f.element == "state_owners"
            || f.element == "controller_flag"
        {
            continue;
        }
        for root in roots.iter_mut() {
            bind::apply::set_element_text(root, &f.element, &f.value);
        }
    }
}

fn paint_state_flags(
    renderer: &mut Renderer,
    panel: &[Node],
    report: &StateBindReport,
    flag_res: &FlagResolver,
) {
    if let (Some(tag), Some(rect)) = (&report.owner_tag, find_rect(panel, "state_owners")) {
        if let Some(rf) = flag_res.resolve(tag, None) {
            if let Ok(tex) = renderer.load_image(&rf.path) {
                renderer.draw_sprite(&tex, rect.x, rect.y, 41.0, 26.0);
            }
        }
    }
    if let (Some(tag), Some(rect)) = (&report.controller_tag, find_rect(panel, "controller_flag")) {
        if let Some(rf) = flag_res.resolve(tag, None) {
            if let Ok(tex) = renderer.load_image(&rf.path) {
                renderer.draw_sprite(&tex, rect.x, rect.y, 41.0, 26.0);
            }
        }
    }
}

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
) {
    let viewport = match find_rect(panel, "state_building_entries") {
        Some(r) => r,
        None => return,
    };
    let template = match find_template(panel_file, manifest, "state_building_entry") {
        Some(t) => t,
        None => return,
    };
    let mut rows = report.state_buildings.clone();
    rows.extend(report.province_buildings.clone());
    let n = rows.len();
    if n == 0 {
        return;
    }
    let spec = GridSpec::from_gridbox(
        &Node { rect: viewport, ..stub_named("state_building_entries") },
        &template,
        14.0,
    );
    let content = ManifestContent { manifest, measure: |_t, _f| None };
    let mut inst = gridbox::instantiate(&template, n, viewport, &spec, scroll_y, &content);
    for (i, row) in inst.rows.iter_mut().enumerate() {
        let b = &rows[i];
        bind::apply::set_element_text(row, "level", &b.level);
        gridbox::set_row_icon_frame(row, "building_picture", b.frame);
    }
    let mut stats = DrawStats::default();
    for row in &inst.rows {
        draw_window(renderer, row, textures, fonts, &mut stats);
    }
    if let Some(sb) = inst.scrollbar {
        if let Some(t) = textures.get(renderer, "GFX_scrollbar_vertical_bg") {
            let border = manifest
                .get("GFX_scrollbar_vertical_bg")
                .and_then(|e| e.border_size)
                .map(|b| [b[0] as f32, b[1] as f32])
                .unwrap_or([0.0, 52.0]);
            renderer.draw_nine_slice(t, sb.track.x, sb.track.y, sb.track.w, sb.track.h, border);
        }
        if let Some(t) = textures.get(renderer, "GFX_generic_box_96") {
            renderer.draw_sprite(t, sb.thumb.x, sb.thumb.y, sb.thumb.w, sb.thumb.h);
        }
    }
}

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

fn find_template(file: &GuiFile, manifest: &SpriteManifest, name: &str) -> Option<Node> {
    let raw = file.windows.iter().find(|w| w.name.as_deref() == Some(name))?;
    let loc = Loc::new();
    Some(Node::from_raw(raw, manifest, &loc))
}

#[allow(clippy::too_many_arguments)]
fn render_wrapped_description(
    renderer: &mut Renderer,
    panel: &[Node],
    report: &StateBindReport,
    state_name: &str,
    fonts: &mut FontCache,
    textures: &mut TextureCache,
    hour: i64,
    date: &str,
) {
    let cat = report.get("category").map(|f| f.value.clone()).unwrap_or_default();
    let vp = report.get("victory_points").map(|f| f.value.clone()).unwrap_or_default();
    // The description carries the SCRUBBED hour + date so it is visibly re-bound per
    // tick (this is the M6 surface that proves the panel text changed with time).
    let src = "At §Yhour $HOUR$§! (§G$DATE$§!) the state of §Y$STATE$§! is a §G$CAT$§! region holding \
               £victory_points $VP$ victory points. All values are bound from the recorded snapshot \
               for this hour, with no fabricated values: unavailable fields render as a dash.";
    let mut vars = HashMap::new();
    vars.insert("STATE".to_string(), state_name.to_string());
    vars.insert("CAT".to_string(), cat);
    vars.insert("VP".to_string(), vp);
    vars.insert("HOUR".to_string(), hour.to_string());
    vars.insert("DATE".to_string(), date.to_string());

    let default_col = [0.90, 0.90, 0.86, 1.0];
    let exp = expand(src, &vars, default_col);
    let bbox = find_rect(panel, "countrystateview")
        .unwrap_or(Rect { x: 40.0, y: 600.0, w: 440.0, h: 200.0 });
    let wrap_x = bbox.x + 22.0;
    let wrap_w = 410.0_f32;
    let wrap_y0 = bbox.y + 470.0;

    let font_name = "hoi_18mbs";
    let font: BmFont = match fonts.get(renderer, font_name) {
        Some(l) => l.font.clone(),
        None => return,
    };
    let wrapped = wrap(&exp, &font, wrap_w);
    let mut y = wrap_y0;
    for line in &wrapped.lines {
        let mut x = wrap_x;
        for span in &line.spans {
            if let Some(icon) = &span.icon {
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
}

fn map_icon_name(token: &str) -> String {
    match token {
        "victory_points" => "GFX_victorypoint_stateview_bg".to_string(),
        other => format!("GFX_{other}"),
    }
}

fn render_occupation_bars(
    renderer: &mut Renderer,
    panel: &[Node],
    report: &StateBindReport,
    textures: &mut TextureCache,
) {
    let occ = match &report.occupation {
        Some(o) => *o,
        None => return,
    };
    let base = find_rect(panel, "resistance_container")
        .unwrap_or(Rect { x: 60.0, y: 760.0, w: 519.0, h: 100.0 });
    let bar_w = 81.0_f32;
    let bar_h = 14.0_f32;
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
