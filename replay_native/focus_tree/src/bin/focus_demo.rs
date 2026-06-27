//! STAGE 3 deliverable — render GER's focus tree to an offscreen PNG showing
//! completed / current / available / locked node states + prerequisite +
//! mutually-exclusive link lines, with the offset-trigger evaluator bounded +
//! badged. READ-ONLY: reads the focus script (mod over vanilla), the snapshot
//! store, and font assets only; never writes the game or a save.
//!
//! Output: replay_native/render/out/focus_tree.png
//! Paths via CLI/env/documented defaults — no personal literal in source.

use std::path::{Path, PathBuf};

use assets::resolver::GameResolver;
use focus_tree::view::{draw_tree, white_texture, Camera, DrawAssets};
use focus_tree::{eval_context, load_catalog, place, FocusDomain, PosFidelity};
use gui_runtime::fonts::{FontCache, FontMap};
use render::{save_png, Renderer};
use snapshot::Store;

fn arg(flag: &str) -> Option<String> {
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

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("render")
        .join("out")
}

fn main() {
    let tag = arg("--country").unwrap_or_else(|| "GER".to_string());
    let tree_id = arg("--tree").unwrap_or_else(|| "german_focus".to_string());
    let game = arg("--game").unwrap_or_else(|| "campaign".to_string());
    let width: u32 = arg("--width").and_then(|s| s.parse().ok()).unwrap_or(1920);
    let height: u32 = arg("--height").and_then(|s| s.parse().ok()).unwrap_or(1080);

    let vanilla = arg("--vanilla");
    // default mod = the sibling AMSB test mod, if present (where german_focus lives)
    let mod_root = arg("--mod").or_else(|| {
        let p = repo_root().parent()?.join("test");
        p.join("common")
            .join("national_focus")
            .is_dir()
            .then(|| p.to_string_lossy().into_owned())
    });
    let resolver = GameResolver::new(vanilla.as_deref(), mod_root.as_deref());

    eprintln!("[focus_demo] vanilla = {}", resolver.vanilla.display());
    if let Some(m) = &resolver.mod_root {
        eprintln!("[focus_demo] mod     = {}", m.display());
    }
    if !resolver.vanilla_exists() {
        eprintln!("[focus_demo] BLOCKER: game install not found. Pass --vanilla <path> or set BATTLE_PLAN_VANILLA.");
        std::process::exit(2);
    }

    // 1) Parse the focus catalog and select the tree.
    let cat = match load_catalog(&resolver) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[focus_demo] BLOCKER: cannot read national_focus dir: {e}");
            std::process::exit(2);
        }
    };
    let tree = match cat.tree(&tree_id) {
        Some(t) => t,
        None => {
            eprintln!(
                "[focus_demo] BLOCKER: tree `{tree_id}` not found ({} trees loaded).",
                cat.trees.len()
            );
            std::process::exit(2);
        }
    };
    eprintln!(
        "[focus_demo] tree `{}`: {} nodes",
        tree.id,
        tree.nodes.len()
    );

    // 2) Bind the snapshot focus domain (real completed/current/progress).
    let store_root = arg("--store")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("replay").join("snapshots"));
    let store = Store::open(&store_root);
    let mut domain = FocusDomain::default();
    let mut date = String::new();
    if let Ok(idx) = store.index(&game) {
        if let Some(hi) = arg("--hour")
            .and_then(|s| s.parse::<i64>().ok())
            .or_else(|| idx.snapshots.iter().find_map(|e| e.hour_index))
        {
            if let Ok(snap) = store.load(&game, hi) {
                date = snap.date.clone().unwrap_or_default();
                if let Some(c) = snap.country(&tag) {
                    domain = FocusDomain::from_country(c);
                }
            }
        }
    }
    if domain.focus_tree.is_none() {
        eprintln!("[focus_demo] WARNING: no focus domain bound for {tag} in game `{game}` — states render as available/locked from prereqs only (no fabrication).");
    } else {
        eprintln!(
            "[focus_demo] bound {} {}: completed={} current={:?} progress={:?}",
            game,
            date,
            domain.completed.len(),
            domain.current,
            domain.progress
        );
    }

    // 3) Place the tree (relative chains + offset evaluator).
    let ctx = eval_context(&tag, &domain, None);
    let placed = place(&tree, &ctx);
    let approx: usize = placed
        .nodes
        .iter()
        .filter(|n| matches!(n.fidelity, PosFidelity::Approximate { .. }))
        .count();
    eprintln!(
        "[focus_demo] placed: {} nodes, {} approximate-position (badged)",
        placed.nodes.len(),
        approx
    );

    // 4) Render offscreen.
    let mut renderer = match Renderer::new(width, height) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[focus_demo] BLOCKER: GPU init failed: {e}");
            std::process::exit(3);
        }
    };
    let white = white_texture(&renderer);

    // font for node labels + legend (best-effort; PNG still renders without it).
    // Fetch ONCE and hold the immutable borrow for the whole draw (FontCache::get
    // is &mut self; the pages/glyphs live inside the cache, not cloneable).
    let font_map = FontMap::from_core_gfx(&resolver);
    let mut fonts = FontCache::new(&font_map, &resolver);
    let font_name = "hoi_16mbs";
    let loaded = fonts.get(&renderer, font_name);
    if loaded.is_none() {
        eprintln!("[focus_demo] note: font `{font_name}` unavailable; rendering without labels.");
    }
    let (font_ref, pages_ref): (Option<&assets::BmFont>, &[render::Texture]) = match &loaded {
        Some(l) => (Some(&l.font), &l.pages),
        None => (None, &[]),
    };

    // viewport = full frame minus a top legend strip
    let legend_h = 64.0;
    let viewport = [0.0, legend_h, width as f32, height as f32 - legend_h];
    let cam = Camera::fit(&placed, viewport, 24.0);

    let assets = DrawAssets {
        white: &white,
        font: font_ref,
        font_pages: pages_ref,
    };
    let stats = draw_tree(&mut renderer, &cam, &placed, &domain, &assets);

    // legend strip (state colours + title).
    draw_legend(
        &mut renderer,
        &white,
        font_ref,
        pages_ref,
        width,
        legend_h,
        &tree.id,
        &tag,
        &date,
        &stats,
    );

    let rgba = renderer.finish([0.07, 0.08, 0.10, 1.0]);
    let out = out_dir();
    let _ = std::fs::create_dir_all(&out);
    let path = out.join("focus_tree.png");
    save_png(&path, width, height, &rgba).expect("save png");

    eprintln!(
        "[focus_demo] wrote {} ({}x{}) — nodes={} links={} excl={} | completed={} current={} available={} locked={} approx={}",
        path.display(), width, height,
        stats.nodes_drawn, stats.links_drawn, stats.exclusives_drawn,
        stats.completed, stats.current, stats.available, stats.locked, stats.approximate
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_legend(
    r: &mut Renderer,
    white: &render::Texture,
    font: Option<&assets::BmFont>,
    pages: &[render::Texture],
    width: u32,
    h: f32,
    tree_id: &str,
    tag: &str,
    date: &str,
    stats: &focus_tree::FocusDrawStats,
) {
    let clip = [0.0, 0.0, width as f32, h];
    r.draw_sprite_frame_clipped(
        white,
        0.0,
        0.0,
        width as f32,
        h,
        0,
        1,
        [0.04, 0.05, 0.07, 1.0],
        Some(clip),
    );
    let chips = [
        (
            [0.85, 0.69, 0.22, 1.0],
            format!("completed {}", stats.completed),
        ),
        (
            [0.20, 0.80, 0.95, 1.0],
            format!("current {}", stats.current),
        ),
        (
            [0.35, 0.55, 0.38, 1.0],
            format!("prereqs-met {}", stats.available),
        ),
        (
            [0.28, 0.30, 0.34, 1.0],
            format!("prereqs-unmet {}", stats.locked),
        ),
        (
            [0.95, 0.15, 0.85, 1.0],
            format!("approx-pos {}", stats.approximate),
        ),
    ];
    if let Some(f) = font {
        let title = format!("FOCUS TREE  {tree_id}   {tag}   {date}");
        r.draw_text_clipped(
            f,
            pages,
            &title,
            12.0,
            8.0,
            [0.95, 0.95, 0.92, 1.0],
            Some(clip),
        );
        let mut x = 12.0;
        let y = 36.0;
        for (c, label) in &chips {
            r.draw_sprite_frame_clipped(white, x, y + 2.0, 14.0, 14.0, 0, 1, *c, Some(clip));
            r.draw_text_clipped(
                f,
                pages,
                label,
                x + 18.0,
                y,
                [0.9, 0.9, 0.88, 1.0],
                Some(clip),
            );
            x += 18.0 + 11.0 * label.len() as f32 + 16.0;
        }
    } else {
        // no font: just colour chips
        let mut x = 12.0;
        for (c, _) in &chips {
            r.draw_sprite_frame_clipped(white, x, 24.0, 20.0, 20.0, 0, 1, *c, Some(clip));
            x += 28.0;
        }
    }
}
