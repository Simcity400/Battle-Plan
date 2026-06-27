//! m2_demo — load the REAL parsed `topbar.json` via `gui_runtime`, lay it out at
//! real game coordinates, draw EVERY element (sprites, 9-slice chrome, BMFont
//! text) STATICALLY via the `render` crate, and write a PNG of the real topbar.
//!
//! No snapshot binding (that is M3): text shows the `.gui` default frames /
//! placeholder values, loc keys are resolved to their localized strings.
//!
//! Game path: `--vanilla <path>` / `BATTLE_PLAN_VANILLA` / the documented Steam default.
//! Parsed-gui / manifest path: `--assets <dir>` / `REPLAY_ASSETS` / the repo's
//! `game_assets` (found by walking up from CWD). READ-ONLY on the game install;
//! writes only the PNG under render/out/.

use std::path::{Path, PathBuf};

use assets::resolver::GameResolver;
use assets::SpriteManifest;
use gui_runtime::draw::{draw_window, DrawStats, TextureCache};
use gui_runtime::fonts::{FontCache, FontMap};
use gui_runtime::{build_scene, hit_test, layout, GuiFile, Loc, ManifestContent, Rect};
use render::{save_png, Renderer};

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

/// Find the repo's `game_assets` dir by walking up from CWD (or override).
fn find_assets_dir() -> Option<PathBuf> {
    if let Some(a) = arg_value("--assets").or_else(|| std::env::var("REPLAY_ASSETS").ok()) {
        return Some(PathBuf::from(a));
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let cand = dir.join("game_assets");
        if cand.join("gui").join("topbar.json").is_file() {
            return Some(cand);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn main() {
    let vanilla = arg_value("--vanilla");
    let mod_root = arg_value("--mod");
    let resolver = GameResolver::new(vanilla.as_deref(), mod_root.as_deref());

    eprintln!("[m2] vanilla = {}", resolver.vanilla.display());
    if !resolver.vanilla_exists() {
        eprintln!(
            "[m2] BLOCKER: game install not found at {}.\n\
             Pass --vanilla <path> or set BATTLE_PLAN_VANILLA.",
            resolver.vanilla.display()
        );
        std::process::exit(2);
    }

    let assets_dir = match find_assets_dir() {
        Some(d) => d,
        None => {
            eprintln!("[m2] BLOCKER: game_assets/gui/topbar.json not found (pass --assets <dir>).");
            std::process::exit(2);
        }
    };
    eprintln!("[m2] assets = {}", assets_dir.display());

    // --- Load the parsed gui, the sprite manifest, and the loc layer. -------
    let gui_path = assets_dir.join("gui").join("topbar.json");
    let manifest_path = assets_dir.join("sprites.json");
    let file = GuiFile::load(&gui_path).expect("BLOCKER: parse topbar.json");
    let manifest = SpriteManifest::load(&manifest_path).expect("BLOCKER: load sprites.json");
    eprintln!(
        "[m2] topbar.json windows={} sprites_manifest={}",
        file.windows.len(),
        manifest.sprites.len()
    );

    // Loc: the game's English localisation (+ mod overlay if present).
    let mut loc = Loc::new();
    let mut loc_keys = 0;
    for base in loc_dirs(&resolver) {
        loc_keys += loc.add_dir(&base).unwrap_or(0);
    }
    eprintln!("[m2] loc keys = {loc_keys}");

    // --- Build the scene-graph and lay it out at a real screen size. --------
    // Layout uses a real 1920x1080 screen so percent / UPPER_RIGHT / LOWER_LEFT
    // anchoring all resolve to the positions the GAME would use. The topbar band
    // lives in the top ~90 px; we render the full screen height so bottom- and
    // right-anchored elements land where the game puts them.
    let (screen_w, screen_h) = (1920u32, 1080u32);
    let screen = Rect {
        x: 0.0,
        y: 0.0,
        w: screen_w as f32,
        h: screen_h as f32,
    };
    let mut scene = build_scene(&file, &manifest, &loc);
    let total_nodes: usize = scene.iter().map(|n| n.count()).sum();
    eprintln!("[m2] scene-graph nodes = {total_nodes}");

    let mut renderer = Renderer::new(screen_w, screen_h).expect("BLOCKER: renderer init");
    eprintln!(
        "[m2] wgpu backend = {:?}, adapter = {} ({:?}), BC = {}",
        renderer.adapter_info.backend,
        renderer.adapter_info.name,
        renderer.adapter_info.device_type,
        renderer.bc_supported
    );

    // Font registry from core.gfx, fonts loaded lazily.
    let font_map = FontMap::from_core_gfx(&resolver);
    let mut fonts = FontCache::new(&font_map, &resolver);
    let mut textures = TextureCache::new(&manifest, &resolver);

    // A measurement closure for content-size resolution (loads a font once).
    // Text content-size needs glyph widths; we pre-measure via a throwaway pass
    // using the font cache, but layout's ContentSize trait wants a plain fn — so
    // we resolve text width to its maxWidth (always present in topbar text) and
    // fall back to 0 (the draw pass measures exactly for alignment).
    let content = ManifestContent {
        manifest: &manifest,
        measure: |_t: &str, _f: &str| None,
    };
    for root in scene.iter_mut() {
        layout(root, screen, None, &content);
    }

    // --- Draw every window in stack/declaration order. ----------------------
    let mut stats = DrawStats::default();
    for root in &scene {
        draw_window(&mut renderer, root, &mut textures, &mut fonts, &mut stats);
    }
    eprintln!(
        "[m2] draws: sprites={} nine_slice={} bars={} text={} containers={} (missing tex={}, missing font={})",
        stats.sprites,
        stats.nine_slices,
        stats.progress_bars,
        stats.text_runs,
        stats.containers,
        stats.missing_textures,
        stats.missing_fonts
    );

    // --- Render + crop to the topbar band for a focused PNG. ----------------
    // The full-screen render is composited; we PNG the top band (0..topbar_h)
    // where the topbar lives, plus a margin, so the verifiable image is the bar.
    let pixels = renderer.finish([0.05, 0.06, 0.08, 1.0]);
    let band_h = 96u32.min(screen_h);
    let cropped = crop_top(&pixels, screen_w, screen_h, band_h);
    let out_path = out_png_path();
    save_png(&out_path, screen_w, band_h, &cropped).expect("write png");

    // Sanity: count drawn pixels in the band.
    let drawn = cropped
        .chunks_exact(4)
        .filter(|p| !(p[0] < 20 && p[1] < 24 && p[2] < 28))
        .count();
    eprintln!(
        "[m2] wrote {} ({}x{}, {} non-bg px)",
        out_path.display(),
        screen_w,
        band_h,
        drawn
    );

    // --- Hit-test spot checks (the interaction foundation). -----------------
    // pol_power_value_bg is at UPPER_LEFT (100,5); the flag at (12,14). Probe a
    // few real element centres and report which node the front-to-back walk hits.
    for (label, x, y) in [
        ("flag area", 30.0, 30.0),
        ("pol_power_bg area", 120.0, 14.0),
        ("right-side band", (screen_w as f32) - 200.0, 20.0),
    ] {
        let hit = scene.iter().rev().find_map(|w| hit_test(w, x, y));
        eprintln!(
            "[m2] hit ({label} @ {x:.0},{y:.0}) -> {}",
            hit.as_ref()
                .and_then(|h| h.leaf())
                .unwrap_or("<none>")
        );
    }

    if drawn < 5000 {
        eprintln!("[m2] WARNING: suspiciously few drawn pixels — render may be blank");
        std::process::exit(3);
    }
    println!("OK {}", out_path.display());
}

/// Crop the top `band_h` rows out of a tightly-packed RGBA8 image.
fn crop_top(pixels: &[u8], w: u32, _h: u32, band_h: u32) -> Vec<u8> {
    let row = (w * 4) as usize;
    pixels[..row * band_h as usize].to_vec()
}

/// Candidate English localisation directories (mod overlay first, then vanilla).
fn loc_dirs(resolver: &GameResolver) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(m) = &resolver.mod_root {
        v.push(m.join("localisation").join("english"));
        v.push(m.join("localisation").join("replace").join("english"));
    }
    v.push(resolver.vanilla.join("localisation").join("english"));
    v
}

fn out_png_path() -> PathBuf {
    // render/out/m2_topbar_static.png, relative to the render crate.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .parent()
        .unwrap()
        .join("render")
        .join("out")
        .join("m2_topbar_static.png")
}
