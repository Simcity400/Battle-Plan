//! Per-frame COMPOSITION onto the offscreen target — BATCH 0: a GENERIC host pass with
//! ZERO per-view branches.
//!
//! The old compose hardcoded the topbar / state-panel / focus / combat draws. Now it
//! iterates the host's bound views in z-order (each self-draws via its `Drawable`) and
//! adds only the TRANSPORT chrome (the scrubber) that is not a content view. There is
//! no `match WindowKind` / `match view_id` here — adding a view never touches this file.
//!
//! Render path unchanged: the map view (Background band, drawn first) CLEARS the target
//! via `render_to_offscreen`; every later view accumulates quads composited OVER it by
//! `flush_no_readback`. Textures + fonts are cached across frames (keyed by RESOLVED
//! sprite/font NAME — the Codex §4 cache-key correctness point).

use std::path::Path;

use gui_runtime::draw::TextureCache;
use gui_runtime::fonts::FontCache;
use gui_runtime::Loc;
use render::Renderer;

use crate::host::draw::{FrameDraw, RenderContext};
use crate::host::view::BoundView;
use crate::host::{Stage, WindowHost};

// Re-export the Stage type from its new home so existing `compose::Stage` paths keep
// working during the migration (the binary + tools refer to it).
pub use crate::host::context::Stage as StageAlias;

/// The transport chrome geometry + the active map-mode HUD string. The map HUD is drawn
/// by the MAP VIEW itself now (it owns its badge); the host transport carries the
/// scrubber track + thumb AND the BATCH-1 MENUBAR plan (the menubar is host chrome —
/// drawn above all views, hit-tested before the host — not a registered view).
pub struct FrameChrome {
    pub scrubber_track: Option<gui_runtime::Rect>,
    pub scrubber_thumb_x: Option<f32>,
    /// The pre-built menubar render plan for this frame + the white texture it tints +
    /// the font name to draw its labels with. `None` = no menubar this frame.
    pub menubar: Option<MenubarChrome>,
}

/// The menubar's per-frame draw inputs: its plan (built in the binary from the registry
/// model) + the leaked white texture it tints + the BMFont name for its labels.
pub struct MenubarChrome {
    pub plan: widgets::Plan,
    pub white: &'static render::Texture,
    pub font: &'static str,
}

/// Draw the whole frame: the host's bound views in z-order (map first, then windows,
/// then overlays) + the scrubber transport chrome, into the offscreen target. After
/// this returns the binary calls `GpuView::present`.
#[allow(clippy::too_many_arguments)]
pub fn compose_offscreen(
    renderer: &mut Renderer,
    stage: &Stage,
    fonts: &mut FontCache<'static>,
    textures: &mut TextureCache<'static>,
    host: &WindowHost,
    bound: &[BoundView],
    chrome: &FrameChrome,
    screen_w: u32,
    screen_h: u32,
) {
    let rc = RenderContext { screen_w, screen_h };
    {
        let mut fd = FrameDraw::new(renderer, textures, fonts, stage, rc);
        // ALL views self-draw in z-order. The Background-band map view runs first and
        // CLEARS the target; everything else accumulates over it. ZERO per-view branches.
        host.draw_all(bound, &mut fd);
    }

    // the scrubber transport (drawn last, always on top) — not a content view.
    draw_scrubber(renderer, textures, chrome);

    // the BATCH-1 MENUBAR (host chrome, above everything). Executes the pre-built plan
    // through the shared `widgets` substrate, resolving label fonts from the cache.
    if let Some(mb) = &chrome.menubar {
        // resolve the label font (a `&LoadedFont` borrows `fonts`; the borrow persists
        // through `execute`, so we keep it scoped). `Texture` is not Clone, so we BORROW
        // the font + its pages — never clone them.
        match fonts.get(renderer, mb.font) {
            Some(loaded) => {
                let assets = widgets::common::DrawAssets {
                    white: mb.white,
                    font: Some(&loaded.font),
                    font_pages: &loaded.pages,
                };
                mb.plan.execute(renderer, &assets, |_, _| false);
            }
            None => {
                // no font yet: still draw the bar geometry (rects); labels skipped.
                let assets = widgets::common::DrawAssets {
                    white: mb.white,
                    font: None,
                    font_pages: &[],
                };
                mb.plan.execute(renderer, &assets, |_, _| false);
            }
        }
    }

    // composite the accumulated quads OVER the map, no clear, NO readback.
    renderer.flush_no_readback(None);
}

/// Draw the scrubber widget — byte-identical to the old `draw_scrubber`.
fn draw_scrubber(renderer: &mut Renderer, textures: &mut TextureCache, chrome: &FrameChrome) {
    let track = match chrome.scrubber_track {
        Some(t) => t,
        None => return,
    };
    let box_tex = match textures.get(renderer, "GFX_generic_box_96") {
        Some(_) => "GFX_generic_box_96",
        None => return,
    };
    if let Some(t) = textures.get(renderer, box_tex) {
        renderer.draw_sprite_frame(
            t,
            track.x,
            track.y,
            track.w,
            track.h,
            0,
            1,
            [0.18, 0.20, 0.24, 1.0],
        );
    }
    if let Some(thumb_x) = chrome.scrubber_thumb_x {
        let fill_w = (thumb_x - track.x).max(0.0);
        if let Some(t) = textures.get(renderer, box_tex) {
            renderer.draw_sprite_frame(
                t,
                track.x,
                track.y,
                fill_w,
                track.h,
                0,
                1,
                [0.45, 0.62, 0.85, 1.0],
            );
        }
        let tw = 8.0_f32;
        let th = track.h + 8.0;
        if let Some(t) = textures.get(renderer, box_tex) {
            renderer.draw_sprite_frame(
                t,
                thumb_x - tw * 0.5,
                track.y - 4.0,
                tw,
                th,
                0,
                1,
                [0.95, 0.95, 0.90, 1.0],
            );
        }
    }
}

/// Resolve a state's display name from `state_meta.json` + loc (kept here; the state
/// panel view + the binary both call it).
pub fn resolve_state_name(map_dir: &Path, loc: &Loc, sid: u32) -> Option<String> {
    let txt = std::fs::read_to_string(map_dir.join("state_meta.json")).ok()?;
    let meta: std::collections::HashMap<String, serde_json::Value> =
        serde_json::from_str(&txt).ok()?;
    let name_key = meta.get(&sid.to_string())?.get("name_key")?.as_str()?;
    Some(loc.resolve(name_key))
}
