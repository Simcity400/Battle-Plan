//! Headless frame dump: render one timeline hour into an offscreen D3D11
//! target and write it as a BMP. No window, no screen interaction.
//!
//! This is the screen-free verification path for the map renderer and the
//! seed of the visual-QA harness (MAP_REPLICA_PLAN Phase 6): fixed-camera
//! renders that can be diffed across hours, builds, and eventually against
//! real the game screenshots.

use std::io::Write;
use std::path::Path;

use map_viewer::GameFileResolver;

use crate::bundle::MapBundle;
use crate::camera::Camera;
use crate::d3d::Gfx;
use crate::overlay::Overlay;
use crate::render::{validate_day_night_hour, MapRenderer, POLITICAL_COLOR_BLOCKER_BADGE};
use crate::timeline::build_provider;
use crate::{Result, TimelineMode};

/// Exact quarter of the 5632x2048 map: full-map fit with zero letterbox,
/// so every pixel is map content and dumps diff cleanly.
const DUMP_WIDTH: u32 = 1408;
const DUMP_HEIGHT: u32 = 512;

/// Render `hour` (default: the timeline's first hour) at whole-map fit,
/// draw the same overlay as the interactive window (text only when the
/// game fonts load from `game_resolver`), and write a 32-bit BMP to `out`.
pub fn dump_frame(
    bundle: &MapBundle,
    mode: &TimelineMode,
    hour: Option<i64>,
    out: &Path,
    game_resolver: Option<&GameFileResolver>,
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&Path>,
    capture_dir: Option<&Path>,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    day_night_hour_override: Option<f32>,
    water_full: bool,
) -> Result<()> {
    let (provider, label) = build_provider(bundle, mode)?;
    let hours = provider.hours();
    if hours.is_empty() {
        return Err("timeline provider has no hours".into());
    }
    let hour = hour.unwrap_or(hours[0]);

    let gfx = Gfx::create_offscreen(DUMP_WIDTH, DUMP_HEIGHT)?;
    if let Some(day_night_hour) = day_night_hour_override {
        validate_day_night_hour(day_night_hour)?;
    }
    let mut renderer = MapRenderer::new(
        &gfx,
        bundle,
        bundle.max_state_id(),
        capture_dir,
        game_resolver,
        postfx_enabled,
        postfx_restore_full,
        water_full,
    )?;
    let mut overlay = Overlay::create(&gfx, game_resolver);
    let owners = provider.owners_at(hour);
    let controllers = provider.controllers_at(hour);
    renderer.update_state_politics(&gfx, &owners, &controllers, &bundle.country_colors)?;
    let mut camera = Camera::fit(
        bundle.width,
        bundle.height,
        f64::from(DUMP_WIDTH),
        f64::from(DUMP_HEIGHT),
    );
    if let Some(zoom) = dump_zoom {
        camera.zoom = zoom;
    }
    if let Some((x, y)) = dump_center {
        camera.center_x = x;
        camera.center_y = y;
    }
    camera.normalize(f64::from(DUMP_HEIGHT));
    // Day/night terminator shading is opt-in via --hour; the default keeps
    // the captured CB0 state (off), matching the game's default map view.
    let day_night_hour = day_night_hour_override;
    renderer.render(&gfx, &camera, day_night_hour, None)?;
    // COLOR-lane debug readback (--dump-hdr): the rendered HDR MainScene
    // (captured-pass RGBA16F output, capture-native dimensions, BEFORE the
    // final composite/postfx). Never a render input.
    if let Some(hdr_path) = dump_hdr {
        renderer.dump_hdr_mainscene(&gfx, hdr_path)?;
    }
    // Same overlay as app.rs so the dump verifies the real product path:
    // line 1 the raw Clausewitz date + hour (GAP: localisation-formatted
    // dates are a later slice), line 2 mode + play state + probe. Headless
    // has no cursor, so the probe shows the empty readout.
    let hour_label = provider.hour_label(hour);
    let hour_index = match hours.binary_search(&hour) {
        Ok(index) => index,
        Err(0) => 0,
        Err(index) => index - 1,
    };
    let line2 =
        format!("{label} | {POLITICAL_COLOR_BLOCKER_BADGE} | prov - state - owner - controller -");
    overlay.draw(&gfx, &hour_label, &line2, hour_index, hours.len())?;
    let pixels = gfx.read_target()?;
    write_bmp(out, DUMP_WIDTH, DUMP_HEIGHT, &pixels)?;
    let day_night_label = match day_night_hour {
        Some(hour) => format!("{hour:.6}"),
        None => "off".to_string(),
    };
    println!(
        "wrote frame dump: {} ({label}, {}, {DUMP_WIDTH}x{DUMP_HEIGHT}, zoom {:.3}, center {:.1},{:.1}, daynight {day_night_label}, postfx {}, restore_full {}, {} controlled states, \
         overlay text {})",
        out.display(),
        hour_label,
        camera.zoom,
        camera.center_x,
        camera.center_y,
        if postfx_enabled { "on" } else { "off" },
        if postfx_restore_full { "on" } else { "off" },
        controllers.len(),
        if overlay.has_text() { "on" } else { "off" }
    );
    if let Some(stats) = renderer.captured_pass_stats() {
        let stats_day_night = match stats.last_day_night_hour {
            Some(hour) => format!("{hour:.6}"),
            None => "off".to_string(),
        };
        println!(
            "captured pass: session {}, terrain {}x{} format {}, water {}, rivers {}, trees {}, postfx {}, restore_full {}, restore_evidence {}, daynight {stats_day_night}, last render {:.2} ms",
            stats.session_dir.display(),
            stats.target_width,
            stats.target_height,
            stats.target_format,
            if stats.has_water { "on" } else { "off" },
            if stats.has_rivers { "on" } else { "off" },
            if stats.has_trees { "on" } else { "off" },
            if stats.postfx_enabled { "on" } else { "off" },
            if stats.postfx_restore_full { "on" } else { "off" },
            if stats.restore_evidence_found { "found" } else { "none" },
            stats.last_render_ms.unwrap_or(0.0)
        );
        if let Some(water_gap) = &stats.water_gap {
            println!("captured pass water gap: {water_gap}");
        }
        for gap in &stats.layer_gaps {
            println!("captured pass layer gap: {gap}");
        }
    }
    Ok(())
}

/// Minimal 32-bit BI_RGB BMP writer. `pixels` are top-down BGRA rows (the
/// readback layout); BMP stores rows bottom-up.
fn write_bmp(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<()> {
    let row_bytes = width as usize * 4;
    if pixels.len() != row_bytes * height as usize {
        return Err(format!(
            "pixel buffer is {} bytes, expected {}",
            pixels.len(),
            row_bytes * height as usize
        )
        .into());
    }
    let image_bytes = pixels.len() as u32;
    let file_bytes = 54 + image_bytes;
    let mut out = Vec::with_capacity(file_bytes as usize);
    // BITMAPFILEHEADER
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&file_bytes.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    out.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
                                                 // BITMAPINFOHEADER
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&image_bytes.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // palette
    out.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..height as usize).rev() {
        out.extend_from_slice(&pixels[y * row_bytes..(y + 1) * row_bytes]);
    }
    let mut file =
        std::fs::File::create(path).map_err(|err| format!("create {}: {err}", path.display()))?;
    file.write_all(&out)
        .map_err(|err| format!("write {}: {err}", path.display()))?;
    Ok(())
}
