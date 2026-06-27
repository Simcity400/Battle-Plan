//! m1_demo — renders the three M1 primitives from REAL the game assets to an
//! offscreen target and writes a verifiable PNG.
//!
//!   (a) a framed sprite from a real `.dds`
//!   (b) a 9-slice (corneredTileSpriteType) chrome frame from a real `.dds`
//!   (c) a line of real BMFont text (garamond_14)
//!
//! Game path comes from `--vanilla <path>` / `BATTLE_PLAN_VANILLA` / the documented
//! Steam default — never hardcoded into a personal literal. READ-ONLY on the
//! game install; writes only the PNG under render/out/.

use std::path::{Path, PathBuf};

use assets::dds;
use assets::resolver::GameResolver;
use assets::BmFont;
use render::{save_png, Renderer, Texture};

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

fn main() {
    let vanilla = arg_value("--vanilla");
    let mod_root = arg_value("--mod");
    let resolver = GameResolver::new(vanilla.as_deref(), mod_root.as_deref());

    eprintln!("[m1] vanilla = {}", resolver.vanilla.display());
    if !resolver.vanilla_exists() {
        eprintln!(
            "[m1] BLOCKER: game install not found at {}.\n\
             Pass --vanilla <path> or set BATTLE_PLAN_VANILLA.",
            resolver.vanilla.display()
        );
        std::process::exit(2);
    }

    // --- Resolve the three real assets. -------------------------------------
    // (a) sprite: command power icon (BGRA8, 24x22) — a real topbar-class icon.
    let sprite_path = resolver
        .resolve("gfx/interface/command_power_icon.dds")
        .or_else(|| resolver.resolve("gfx/interface/manpower.dds"))
        .expect("BLOCKER: no representative sprite .dds found");
    // (b) 9-slice chrome: tiled_generic_overlay_bg1_small (290x310, border 144).
    let nine_path = resolver
        .resolve("gfx/interface/tiles/tiled_generic_overlay_bg1_small.dds")
        .or_else(|| resolver.resolve("gfx/interface/tiles/tiled_generic_overlay_bg1.dds"))
        .expect("BLOCKER: no 9-slice chrome .dds found");
    let nine_border = [144.0_f32, 144.0]; // borderSize from core.gfx
                                          // (c) font: garamond_14 (the main UI font).
    let fnt_path = resolver
        .resolve("gfx/fonts/garamond_14.fnt")
        .expect("BLOCKER: garamond_14.fnt not found");

    eprintln!("[m1] sprite   = {}", sprite_path.display());
    eprintln!("[m1] 9-slice  = {}", nine_path.display());
    eprintln!("[m1] font     = {}", fnt_path.display());

    // Audit-print the three DDS headers (mini in-binary audit).
    for (label, p) in [("sprite", &sprite_path), ("9-slice", &nine_path)] {
        if let Ok(b) = std::fs::read(p) {
            if let Ok(h) = dds::parse_header(&b) {
                eprintln!(
                    "[m1] dds {label}: {}x{} {} alpha={:?} mips={}",
                    h.width, h.height, h.format, h.alpha, h.mip_count
                );
            }
        }
    }

    // --- Build the offscreen renderer. --------------------------------------
    let (w, h) = (640u32, 360u32);
    let mut r = Renderer::new(w, h).expect("BLOCKER: renderer init failed");
    eprintln!(
        "[m1] wgpu backend = {:?}, adapter = {} ({:?}), BC_supported = {}",
        r.adapter_info.backend, r.adapter_info.name, r.adapter_info.device_type, r.bc_supported
    );

    // --- Load assets to GPU. ------------------------------------------------
    let sprite_tex = r.load_dds(&sprite_path).expect("load sprite dds");
    let nine_tex = r.load_dds(&nine_path).expect("load 9-slice dds");
    eprintln!(
        "[m1] sprite tex {}x{} {} gpu_native={}",
        sprite_tex.width, sprite_tex.height, sprite_tex.format, sprite_tex.gpu_native
    );
    eprintln!(
        "[m1] 9-slice tex {}x{} {} gpu_native={}",
        nine_tex.width, nine_tex.height, nine_tex.format, nine_tex.gpu_native
    );

    let font = BmFont::load(&fnt_path).expect("load garamond_14.fnt");
    // Resolve the page atlases (relative to the .fnt) to GPU textures.
    let page_textures = load_font_pages(&r, &font, &fnt_path);
    eprintln!(
        "[m1] font '{}' lineHeight={} base={} glyphs={} pages={}",
        font.face,
        font.line_height,
        font.base,
        font.glyphs.len(),
        page_textures.len()
    );

    // --- Compose the scene. -------------------------------------------------
    // (b) draw the 9-slice chrome frame FIRST (background panel).
    r.draw_nine_slice(&nine_tex, 24.0, 24.0, 360.0, 200.0, nine_border);

    // (a) draw the sprite, scaled 3x, over the panel.
    let sw = sprite_tex.width as f32 * 3.0;
    let sh = sprite_tex.height as f32 * 3.0;
    r.draw_sprite(&sprite_tex, 60.0, 70.0, sw, sh);
    // also draw it at native 1:1 to prove non-stretched crispness
    r.draw_sprite(
        &sprite_tex,
        60.0,
        70.0 + sh + 12.0,
        sprite_tex.width as f32,
        sprite_tex.height as f32,
    );

    // (c) draw BMFont text in two colours, plus a measured-width underline.
    let line1 = "the game";
    let line2 = "M1 render primitives: AVA WAVE 0123";
    let baseline_y = 250.0;
    r.draw_text(
        &font,
        &page_textures,
        line1,
        40.0,
        baseline_y,
        [1.0, 0.9, 0.4, 1.0],
    );
    r.draw_text(
        &font,
        &page_textures,
        line2,
        40.0,
        baseline_y + font.line_height as f32 + 4.0,
        [0.85, 0.92, 1.0, 1.0],
    );
    // Prove measurement: draw a thin underline exactly as wide as the measured
    // resolved string (using the sprite tex as a 1px tinted bar).
    let measured = font.measure(line1) as f32;
    r.draw_sprite_frame(
        &sprite_tex,
        40.0,
        baseline_y + font.line_height as f32 - 1.0,
        measured,
        2.0,
        0,
        1,
        [0.4, 1.0, 0.5, 0.9],
    );
    eprintln!("[m1] measured '{}' = {} px", line1, measured);

    // --- Render + save. -----------------------------------------------------
    let pixels = r.finish([0.06, 0.07, 0.09, 1.0]);
    let out_path = out_png_path();
    save_png(&out_path, w, h, &pixels).expect("write png");

    // Sanity: count non-background pixels so a blank render is caught.
    let drawn = pixels
        .chunks_exact(4)
        .filter(|p| !(p[0] < 24 && p[1] < 28 && p[2] < 32))
        .count();
    eprintln!(
        "[m1] wrote {} ({} non-background px of {})",
        out_path.display(),
        drawn,
        w * h
    );
    if drawn < 1000 {
        eprintln!("[m1] WARNING: suspiciously few drawn pixels — render may be blank");
        std::process::exit(3);
    }
    println!("OK {}", out_path.display());
}

/// Resolve and upload each font page atlas, indexed by page id.
fn load_font_pages(r: &Renderer, font: &BmFont, fnt_path: &Path) -> Vec<Texture> {
    let max_page = font.page_paths.keys().copied().max().unwrap_or(0) as usize;
    // A 1x1 white placeholder for any unresolved page (keeps indices valid).
    let placeholder = r.upload_rgba8_image(1, 1, &[255, 255, 255, 255]);
    let mut pages: Vec<Texture> = Vec::with_capacity(max_page + 1);
    for id in 0..=max_page {
        let id = id as u16;
        if let Some(p) = font.page_paths.get(&id) {
            // Font atlases are `.dds` (BGRA8) in the game's English set.
            match r.load_dds(p) {
                Ok(t) => pages.push(t),
                Err(e) => {
                    eprintln!("[m1] WARN: font page {id} {} load failed: {e}", p.display());
                    pages.push(r.upload_rgba8_image(1, 1, &[255, 255, 255, 255]));
                }
            }
        } else {
            eprintln!(
                "[m1] WARN: font page {id} unresolved near {}",
                fnt_path.display()
            );
            pages.push(r.upload_rgba8_image(1, 1, &[255, 255, 255, 255]));
        }
    }
    if pages.is_empty() {
        pages.push(placeholder);
    }
    pages
}

fn out_png_path() -> PathBuf {
    // render/out/m1_primitives.png, relative to this crate.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .join("out")
        .join("m1_primitives.png")
}
