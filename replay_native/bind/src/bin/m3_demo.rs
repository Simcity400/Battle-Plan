//! m3_demo — bind the M2 topbar to a REAL recorded snapshot and render the BOUND
//! topbar to a PNG. Loads `topbar.json` (gui_runtime) + a snapshot (snapshot
//! crate) + applies the `bind` crate's topbar spec, substituting GER's real
//! values, resolving the ideology-specific flag, and rendering the unavailable
//! elements as the not-captured glyph ("—"), never a fabricated 0.
//!
//! CLI (all optional, defaults target the committed demo frame):
//!   --country <TAG>     (default GER)
//!   --game    <id>      (default replay)
//!   --hour    <i64>     (default 17005619 = 1941.4.13.11)
//!   --store   <dir>     (default: ../replay/snapshots found by walking up)
//!   --vanilla <path> / --mod <path> / --assets <dir>   (as m2_demo)
//!
//! READ-ONLY on the game install + the snapshot store; writes only the PNG under
//! render/out/. No personal path is baked in — all locations come via CLI/env.

use std::path::{Path, PathBuf};

use assets::resolver::GameResolver;
use assets::SpriteManifest;
use bind::{apply_topbar, FlagResolver, TopbarSpec};
use gui_runtime::draw::{draw_window, DrawStats, TextureCache};
use gui_runtime::fonts::{FontCache, FontMap};
use gui_runtime::scene::Node;
use gui_runtime::{build_scene, layout, GuiFile, Loc, ManifestContent, Rect};
use render::{save_png, Renderer, Texture};
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

/// Find the snapshot store root (`<...>/replay/snapshots`) by walking up.
fn find_store_root() -> Option<PathBuf> {
    if let Some(s) = arg_value("--store").or_else(|| std::env::var("REPLAY_STORE").ok()) {
        return Some(PathBuf::from(s));
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let cand = dir.join("replay").join("snapshots");
        if cand.join("replay").join("index.json").is_file() {
            return Some(cand);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn main() {
    // --- CLI args (defaults = the committed demo frame). --------------------
    let country = arg_value("--country").unwrap_or_else(|| "GER".to_string());
    let game = arg_value("--game").unwrap_or_else(|| "replay".to_string());
    let hour: i64 = arg_value("--hour")
        .and_then(|s| s.parse().ok())
        .unwrap_or(17005619);

    let vanilla = arg_value("--vanilla");
    let mod_root = arg_value("--mod");
    let resolver = GameResolver::new(vanilla.as_deref(), mod_root.as_deref());

    eprintln!("[m3] country={country} game={game} hour={hour}");
    eprintln!("[m3] vanilla = {}", resolver.vanilla.display());
    if !resolver.vanilla_exists() {
        eprintln!(
            "[m3] BLOCKER: game install not found at {}.\n\
             Pass --vanilla <path> or set BATTLE_PLAN_VANILLA.",
            resolver.vanilla.display()
        );
        std::process::exit(2);
    }

    let assets_dir = match find_assets_dir() {
        Some(d) => d,
        None => {
            eprintln!("[m3] BLOCKER: game_assets/gui/topbar.json not found (pass --assets <dir>).");
            std::process::exit(2);
        }
    };
    let store_root = match find_store_root() {
        Some(d) => d,
        None => {
            eprintln!(
                "[m3] BLOCKER: snapshot store (replay/snapshots) not found (pass --store <dir>)."
            );
            std::process::exit(2);
        }
    };
    eprintln!("[m3] assets = {}", assets_dir.display());
    eprintln!("[m3] store  = {}", store_root.display());

    // --- Load the snapshot (the real recorded frame). -----------------------
    let store = Store::open(&store_root);
    let snap = match store.load(&game, hour) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[m3] BLOCKER: load snapshot {game} @ {hour}: {e}");
            std::process::exit(2);
        }
    };
    eprintln!(
        "[m3] snapshot date={} hour={} countries={} states={}",
        snap.date.as_deref().unwrap_or("?"),
        snap.hour_index.unwrap_or(-1),
        snap.countries().map(|m| m.len()).unwrap_or(0),
        snap.states().map(|m| m.len()).unwrap_or(0),
    );
    if snap.country(&country).is_none() {
        eprintln!("[m3] BLOCKER: country {country} not in snapshot.");
        std::process::exit(2);
    }

    // --- Load the parsed gui, the sprite manifest, and the loc layer. -------
    let gui_path = assets_dir.join("gui").join("topbar.json");
    let manifest_path = assets_dir.join("sprites.json");
    let file = GuiFile::load(&gui_path).expect("BLOCKER: parse topbar.json");
    let manifest = SpriteManifest::load(&manifest_path).expect("BLOCKER: load sprites.json");

    let mut loc = Loc::new();
    for base in loc_dirs(&resolver) {
        let _ = loc.add_dir(&base);
    }
    eprintln!(
        "[m3] topbar windows={} sprites={} loc_keys={}",
        file.windows.len(),
        manifest.sprites.len(),
        loc.len()
    );

    // --- Build the scene-graph + APPLY the binding spec (the M3 core). ------
    let mut scene = build_scene(&file, &manifest, &loc);
    let spec = TopbarSpec::topbar();
    let report = apply_topbar(&mut scene, &spec, &snap, &country);

    // Print the bound values (the verifiable report).
    eprintln!("[m3] --- bound topbar values (country={country}) ---");
    for f in &report.fields {
        let badge = if f.not_captured { "NOT-CAPTURED" } else { "" };
        eprintln!(
            "[m3]   {:22} = {:<22} [{:?}] {badge}",
            f.element, f.value, f.fidelity
        );
    }

    // --- Resolve the ideology-specific country flag. ------------------------
    let flag_res = FlagResolver::new(&resolver);
    let resolved_flag = flag_res.resolve(&country, report.flag_ideology.as_deref());
    match &resolved_flag {
        Some(rf) => eprintln!(
            "[m3] flag: {} + {:?} -> {}",
            rf.tag,
            rf.ideology,
            rf.path.display()
        ),
        None => eprintln!("[m3] flag: no flag file for {country} (keeping shield chrome)"),
    }

    // --- Lay out at a real 1920x1080 screen. --------------------------------
    let (screen_w, screen_h) = (1920u32, 1080u32);
    let screen = Rect {
        x: 0.0,
        y: 0.0,
        w: screen_w as f32,
        h: screen_h as f32,
    };
    let content = ManifestContent {
        manifest: &manifest,
        measure: |_t: &str, _f: &str| None,
    };
    for root in scene.iter_mut() {
        layout(root, screen, None, &content);
    }

    // --- Renderer + caches. -------------------------------------------------
    let mut renderer = Renderer::new(screen_w, screen_h).expect("BLOCKER: renderer init");
    eprintln!(
        "[m3] wgpu backend={:?} adapter={} BC={}",
        renderer.adapter_info.backend, renderer.adapter_info.name, renderer.bc_supported
    );
    let font_map = FontMap::from_core_gfx(&resolver);
    let mut fonts = FontCache::new(&font_map, &resolver);
    let mut textures = TextureCache::new(&manifest, &resolver);

    // Pre-load the real flag texture (if resolved) so we can paint it over the
    // player_flag slot AFTER the scene-graph chrome draws.
    let flag_tex: Option<(Rect, Texture)> = resolved_flag.as_ref().and_then(|rf| {
        let rect = find_rect(&scene, "player_flag")?;
        match renderer.load_image(&rf.path) {
            Ok(t) => Some((rect, t)),
            Err(e) => {
                eprintln!("[m3] WARN: load flag {} failed: {e}", rf.path.display());
                None
            }
        }
    });

    // --- Draw the bound topbar. ---------------------------------------------
    let mut stats = DrawStats::default();
    for root in &scene {
        draw_window(&mut renderer, root, &mut textures, &mut fonts, &mut stats);
    }
    // Paint the real flag image into the player_flag slot (over the shield chrome).
    if let Some((r, tex)) = &flag_tex {
        renderer.draw_sprite(tex, r.x, r.y, r.w, r.h);
        eprintln!(
            "[m3] painted flag at ({:.0},{:.0}) {:.0}x{:.0}",
            r.x, r.y, r.w, r.h
        );
    }
    eprintln!(
        "[m3] draws: sprites={} nine={} bars={} text={} (missing tex={}, missing font={})",
        stats.sprites,
        stats.nine_slices,
        stats.progress_bars,
        stats.text_runs,
        stats.missing_textures,
        stats.missing_fonts
    );

    // --- Render + crop to the topbar band. ----------------------------------
    let pixels = renderer.finish([0.05, 0.06, 0.08, 1.0]);
    let band_h = 96u32.min(screen_h);
    let cropped = crop_top(&pixels, screen_w, band_h);
    let out_path = out_png_path();
    save_png(&out_path, screen_w, band_h, &cropped).expect("write png");

    let drawn = cropped
        .chunks_exact(4)
        .filter(|p| !(p[0] < 20 && p[1] < 24 && p[2] < 28))
        .count();
    eprintln!(
        "[m3] wrote {} ({}x{}, {} non-bg px)",
        out_path.display(),
        screen_w,
        band_h,
        drawn
    );

    // --- Verify the no-fabrication invariant in the OUTPUT report. ----------
    let mut nc_ok = true;
    for f in report
        .fields
        .iter()
        .filter(|f| matches!(f.fidelity, bind::Fidelity::Unavailable))
    {
        if !f.not_captured || f.value != "—" {
            eprintln!(
                "[m3] FABRICATION BUG: unavailable {} rendered {:?} (expected —)",
                f.element, f.value
            );
            nc_ok = false;
        }
    }
    if !nc_ok {
        eprintln!("[m3] BLOCKER: a no-fabrication invariant was violated.");
        std::process::exit(3);
    }
    if drawn < 5000 {
        eprintln!("[m3] WARNING: suspiciously few drawn pixels — render may be blank");
        std::process::exit(3);
    }
    eprintln!("[m3] no-fabrication: all Unavailable elements rendered '—' (not 0). OK");
    println!("OK {}", out_path.display());
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

fn crop_top(pixels: &[u8], w: u32, band_h: u32) -> Vec<u8> {
    let row = (w * 4) as usize;
    pixels[..row * band_h as usize].to_vec()
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

fn out_png_path() -> PathBuf {
    // render/out/m3_topbar_bound.png, relative to this crate.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .parent()
        .unwrap()
        .join("render")
        .join("out")
        .join("m3_topbar_bound.png")
}
