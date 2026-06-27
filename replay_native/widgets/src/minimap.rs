//! N7 — the MINIMAP VIEWPORT: a downscaled map panel + the main camera's view rect
//! outlined on it + click-to-recenter. Consumers: the chrome/outliner-maps cluster.
//!
//! Codex N7 hardening: the widget owns ONLY the math + the outline draw (the caller
//! supplies the downscaled map TEXTURE — N7 never fabricates a map background). The
//! world→minimap transform is an explicit ASPECT-FIT (letterbox) so a non-square world
//! is not distorted; a zero/negative/non-finite world size DISABLES picking + drawing
//! (no divide-by-zero). [`Minimap::pick`] returns `Option<WorldPoint>` — a click in the
//! LETTERBOX margin (outside the fitted map) is `None`, never a clamped fabrication.
//! The camera-view rect is clipped to the map area when drawn.
//!
//! ## Usage
//! ```ignore
//! use widgets::minimap::*;
//! let mm = Minimap::new(
//!     gui_runtime::Rect { x: 20.0, y: 600.0, w: 240.0, h: 100.0 }, // panel on screen
//!     5632.0, 2048.0,                                              // world (raster) size
//! );
//! let view = WorldRect { x: 1000.0, y: 400.0, w: 1400.0, h: 700.0 }; // main camera view
//! // draw_minimap(renderer, &mm, view, map_texture, &assets);
//! if let Some(p) = mm.pick(view, mx, my) { /* recenter main camera on p */ }
//! ```

use render::{Renderer, Texture};

use crate::common::{fill_rect, stroke_rect, DrawAssets};
use crate::plan::Plan;

/// A point in WORLD (raster-pixel) space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldPoint {
    pub x: f32,
    pub y: f32,
}

/// A rect in WORLD (raster-pixel) space (the main camera's visible window).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The minimap: a screen panel rect + the world size it represents. The fitted map
/// sub-rect (aspect-fit, letterboxed) is derived in [`Minimap::fitted`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Minimap {
    /// The minimap panel on screen.
    pub panel: gui_runtime::Rect,
    /// The world (raster) dimensions the minimap represents.
    pub world_w: f32,
    pub world_h: f32,
}

impl Minimap {
    pub fn new(panel: gui_runtime::Rect, world_w: f32, world_h: f32) -> Minimap {
        Minimap { panel, world_w, world_h }
    }

    /// True iff the minimap can draw/pick (finite, positive world + panel dims AND a
    /// finite panel origin — Codex: a non-finite panel.x/y would poison the projection).
    pub fn is_usable(&self) -> bool {
        self.world_w.is_finite()
            && self.world_h.is_finite()
            && self.world_w > 0.0
            && self.world_h > 0.0
            && self.panel.x.is_finite()
            && self.panel.y.is_finite()
            && self.panel.w > 0.0
            && self.panel.h > 0.0
    }

    /// The ASPECT-FIT map sub-rect inside the panel (letterboxed): the largest rect with
    /// the world's aspect ratio that fits the panel, centred. `None` if unusable.
    pub fn fitted(&self) -> Option<gui_runtime::Rect> {
        if !self.is_usable() {
            return None;
        }
        let world_aspect = self.world_w / self.world_h;
        let panel_aspect = self.panel.w / self.panel.h;
        let (fw, fh) = if panel_aspect > world_aspect {
            // panel is wider → fit height, letterbox left/right.
            let fh = self.panel.h;
            (fh * world_aspect, fh)
        } else {
            let fw = self.panel.w;
            (fw, fw / world_aspect)
        };
        let fx = self.panel.x + (self.panel.w - fw) / 2.0;
        let fy = self.panel.y + (self.panel.h - fh) / 2.0;
        Some(gui_runtime::Rect { x: fx, y: fy, w: fw, h: fh })
    }

    /// Project a WORLD point to a SCREEN pixel within the fitted map. `None` if unusable.
    pub fn world_to_screen(&self, p: WorldPoint) -> Option<(f32, f32)> {
        let f = self.fitted()?;
        let u = (p.x / self.world_w).clamp(0.0, 1.0);
        let v = (p.y / self.world_h).clamp(0.0, 1.0);
        Some((f.x + u * f.w, f.y + v * f.h))
    }

    /// Project a SCREEN pixel to a WORLD point — ONLY if the click lands inside the
    /// fitted map (a click in the letterbox margin returns `None`, never a clamped
    /// fabrication — Codex N7). `None` if unusable.
    pub fn pick(&self, _camera_view: WorldRect, sx: f32, sy: f32) -> Option<WorldPoint> {
        let f = self.fitted()?;
        if !f.contains(sx, sy) {
            return None; // letterbox / outside → no world point
        }
        let u = (sx - f.x) / f.w;
        let v = (sy - f.y) / f.h;
        Some(WorldPoint { x: u * self.world_w, y: v * self.world_h })
    }

    /// The camera-view rect projected to SCREEN px, clipped to the fitted map. The world
    /// rect may extend past the world bounds (a zoomed-out camera); it is intersected with
    /// the fitted map so the outline never leaks past the minimap. Returns `None` if the
    /// view is unusable, non-finite, OR lies ENTIRELY off the world (an empty intersection)
    /// — Codex: do NOT fabricate a 1px edge marker for an off-world camera.
    pub fn camera_screen_rect(&self, view: WorldRect) -> Option<gui_runtime::Rect> {
        if !(view.x.is_finite() && view.y.is_finite() && view.w.is_finite() && view.h.is_finite()) {
            return None;
        }
        let f = self.fitted()?;
        let p0 = self.world_to_screen(WorldPoint { x: view.x, y: view.y })?;
        let p1 = self.world_to_screen(WorldPoint { x: view.x + view.w, y: view.y + view.h })?;
        // raw projected rect (NOT min-1 padded — an off-world view must be able to clip to
        // empty, not be forced to a 1px sliver).
        let r = gui_runtime::Rect { x: p0.0, y: p0.1, w: (p1.0 - p0.0).max(0.0), h: (p1.1 - p0.1).max(0.0) };
        let clipped = r.intersect(&f);
        if clipped.is_empty() {
            None // entirely off the fitted map → no outline (no fabrication)
        } else {
            Some(clipped)
        }
    }
}

const PANEL_BG: [f32; 4] = [0.05, 0.06, 0.08, 1.0];
const LETTERBOX_BG: [f32; 4] = [0.02, 0.02, 0.03, 1.0];
const CAMERA_OUTLINE: [f32; 4] = [0.95, 0.92, 0.55, 1.0];

/// Build the render plan for the minimap CHROME (panel bg + letterbox + camera outline).
/// The map TEXTURE is drawn by [`draw_minimap`] (the plan can't carry a texture handle);
/// an unusable minimap emits an empty plan.
pub fn plan_minimap(mm: &Minimap, view: WorldRect) -> Plan {
    let mut p = Plan::new();
    if !mm.is_usable() {
        return p;
    }
    let panel = mm.panel;
    let clip = Some([panel.x, panel.y, panel.w, panel.h]);
    // letterbox background fills the whole panel, then the fitted map area is a distinct
    // bg (the caller's texture draws over it).
    p.push(crate::plan::Cmd::Rect { x: panel.x, y: panel.y, w: panel.w, h: panel.h, tint: LETTERBOX_BG, clip });
    if let Some(f) = mm.fitted() {
        p.push(crate::plan::Cmd::Rect { x: f.x, y: f.y, w: f.w, h: f.h, tint: PANEL_BG, clip });
    }
    if let Some(cr) = mm.camera_screen_rect(view) {
        // a 2px outline rect (4 thin fills via stroke at execute via Rect cmds).
        let t = 2.0;
        p.push(crate::plan::Cmd::Rect { x: cr.x, y: cr.y, w: cr.w, h: t, tint: CAMERA_OUTLINE, clip });
        p.push(crate::plan::Cmd::Rect { x: cr.x, y: cr.y + cr.h - t, w: cr.w, h: t, tint: CAMERA_OUTLINE, clip });
        p.push(crate::plan::Cmd::Rect { x: cr.x, y: cr.y, w: t, h: cr.h, tint: CAMERA_OUTLINE, clip });
        p.push(crate::plan::Cmd::Rect { x: cr.x + cr.w - t, y: cr.y, w: t, h: cr.h, tint: CAMERA_OUTLINE, clip });
    }
    p
}

/// Draw the minimap: chrome (plan) + the caller-supplied downscaled map texture over the
/// fitted area. `map` is the downscaled map (caller-owned; N7 never makes one). An
/// unusable minimap draws nothing.
pub fn draw_minimap(
    r: &mut Renderer,
    mm: &Minimap,
    view: WorldRect,
    map: Option<&Texture>,
    assets: &DrawAssets,
) {
    if !mm.is_usable() {
        return;
    }
    let panel = mm.panel;
    let clip = Some([panel.x, panel.y, panel.w, panel.h]);
    // letterbox + fitted bg first.
    fill_rect(r, assets.white, panel.x, panel.y, panel.w, panel.h, LETTERBOX_BG, clip);
    if let Some(f) = mm.fitted() {
        let fclip = Some([f.x, f.y, f.w, f.h]);
        match map {
            Some(tex) => r.draw_sprite_frame_clipped(tex, f.x, f.y, f.w, f.h, 0, 1, [1.0; 4], fclip),
            None => fill_rect(r, assets.white, f.x, f.y, f.w, f.h, PANEL_BG, fclip), // no fabricated map
        }
    }
    // camera outline on top.
    if let Some(cr) = mm.camera_screen_rect(view) {
        stroke_rect(r, assets.white, cr.x, cr.y, cr.w, cr.h, 2.0, CAMERA_OUTLINE, clip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm() -> Minimap {
        // a 240x100 panel for a 5632x2048 world (≈2.75:1) — letterboxed top/bottom.
        Minimap::new(gui_runtime::Rect { x: 20.0, y: 600.0, w: 240.0, h: 100.0 }, 5632.0, 2048.0)
    }

    #[test]
    fn aspect_fit_letterboxes_not_distorts() {
        let m = mm();
        let f = m.fitted().unwrap();
        // panel is wider-aspect than 2.75:1? 240/100=2.4 < 2.75 -> fit WIDTH, letterbox y.
        assert!((f.w - 240.0).abs() < 0.01);
        assert!(f.h < 100.0); // letterboxed vertically
        assert!(f.y > m.panel.y); // centred with top margin
    }

    #[test]
    fn unusable_minimap_disables_everything() {
        let bad = Minimap::new(gui_runtime::Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, 0.0, -5.0);
        assert!(!bad.is_usable());
        assert!(bad.fitted().is_none());
        assert!(bad.pick(WorldRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, 10.0, 10.0).is_none());
        assert!(plan_minimap(&bad, WorldRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }).is_empty());
    }

    #[test]
    fn pick_inside_map_roundtrips_world_point() {
        let m = mm();
        let f = m.fitted().unwrap();
        // centre of the fitted map → world centre.
        let p = m.pick(WorldRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, f.x + f.w / 2.0, f.y + f.h / 2.0).unwrap();
        assert!((p.x - 5632.0 / 2.0).abs() < 1.0);
        assert!((p.y - 2048.0 / 2.0).abs() < 1.0);
    }

    #[test]
    fn pick_in_letterbox_is_none_not_clamped() {
        let m = mm();
        // a click in the top letterbox margin (above the fitted map) → None.
        let click_y = m.panel.y + 1.0; // inside the panel but above fitted (letterbox)
        assert!(m.pick(WorldRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, m.panel.x + 120.0, click_y).is_none());
    }

    #[test]
    fn off_world_camera_draws_no_outline() {
        let m = mm();
        // a camera entirely to the right of the world (x beyond world_w) clamps to the
        // right edge for both corners -> empty intersection -> None (no 1px fabrication).
        let off = WorldRect { x: 99999.0, y: 99999.0, w: 10.0, h: 10.0 };
        assert!(m.camera_screen_rect(off).is_none());
        // a non-finite view also yields None.
        assert!(m.camera_screen_rect(WorldRect { x: f32::NAN, y: 0.0, w: 1.0, h: 1.0 }).is_none());
    }

    #[test]
    fn camera_rect_clips_to_fitted_map() {
        let m = mm();
        // a camera view larger than the world clamps its screen rect to the fitted map.
        let huge = WorldRect { x: -1000.0, y: -1000.0, w: 9000.0, h: 9000.0 };
        let cr = m.camera_screen_rect(huge).unwrap();
        let f = m.fitted().unwrap();
        assert!(cr.x >= f.x - 0.01 && cr.y >= f.y - 0.01);
        assert!(cr.x + cr.w <= f.x + f.w + 0.01);
    }
}
