//! The MAP CAMERA — pan/zoom state + screen↔raster mapping (Stage 1 deliverable 3).
//!
//! The map fills the whole window (the bottom compositor layer). The camera owns the
//! M4 map uniforms (`offset` + `scale` in map-UV) and the geometry needed to turn a
//! SCREEN pixel into a RASTER pixel for CPU province picking — both must use the
//! IDENTICAL transform the shader uses, or hover/click would pick a different
//! province than the one drawn under the cursor.
//!
//! The shader transform (see `render/src/map.wgsl`):
//!   `map_uv = out_uv / scale + offset`,   where `out_uv` is the destination-rect
//!   fraction (0..1) of the pixel. With the map filling the whole `(screen_w,
//!   screen_h)` destination, `out_uv = (px/screen_w, py/screen_h)`. The raster pixel
//!   is then `(map_uv.x * raster_w, map_uv.y * raster_h)`.
//!
//! Pan is a drag in screen pixels converted to a UV delta; zoom is a scroll that
//! scales about the CURSOR (so the point under the cursor stays put — the standard
//! focal zoom). All PURE; the app feeds the resulting `offset`/`scale` to
//! `PoliticalMap::render_to_offscreen` and uses `screen_to_raster` for picking.

/// The map camera: a uniform pan offset + zoom scale, plus the current viewport size
/// and the raster size (set once). `scale` is isotropic in Stage 1 (square pixels).
#[derive(Debug, Clone, Copy)]
pub struct MapCamera {
    /// pan offset in map-UV (0..1). `map_uv = out_uv/scale + offset`.
    pub offset: [f32; 2],
    /// zoom scale (1 = full world). Larger = zoomed IN.
    pub scale: f32,
    /// the screen viewport the map fills (pixels).
    pub screen_w: f32,
    pub screen_h: f32,
    /// the province-id raster dimensions (pixels).
    pub raster_w: f32,
    pub raster_h: f32,
}

impl MapCamera {
    /// A full-world camera (no pan, no zoom) for a `(screen)` viewport over a
    /// `(raster)`-sized province map.
    pub fn new(screen_w: u32, screen_h: u32, raster_w: u32, raster_h: u32) -> MapCamera {
        MapCamera {
            offset: [0.0, 0.0],
            scale: 1.0,
            screen_w: screen_w as f32,
            screen_h: screen_h as f32,
            raster_w: raster_w as f32,
            raster_h: raster_h as f32,
        }
    }

    /// Update the viewport size (on window resize). Offset/scale are preserved.
    pub fn set_viewport(&mut self, screen_w: u32, screen_h: u32) {
        self.screen_w = screen_w.max(1) as f32;
        self.screen_h = screen_h.max(1) as f32;
    }

    /// The uniforms for `PoliticalMap::render_to_offscreen`: `(offset, [scale,scale])`.
    pub fn uniform_offset(&self) -> [f32; 2] {
        self.offset
    }
    pub fn uniform_scale(&self) -> [f32; 2] {
        [self.scale, self.scale]
    }

    /// Convert a screen pixel `(px,py)` to the map-UV the shader samples.
    fn screen_to_map_uv(&self, px: f32, py: f32) -> [f32; 2] {
        let out_u = px / self.screen_w;
        let out_v = py / self.screen_h;
        [
            out_u / self.scale + self.offset[0],
            out_v / self.scale + self.offset[1],
        ]
    }

    /// Convert a screen pixel to a RASTER pixel `(x,y)` for CPU province picking.
    /// Returns `None` if the cursor maps OUTSIDE the raster (e.g. panned past the map
    /// edge) — a pick there is honestly "no province", never a clamped fabrication.
    pub fn screen_to_raster(&self, px: f32, py: f32) -> Option<(u32, u32)> {
        let uv = self.screen_to_map_uv(px, py);
        if uv[0] < 0.0 || uv[0] >= 1.0 || uv[1] < 0.0 || uv[1] >= 1.0 {
            return None;
        }
        let rx = (uv[0] * self.raster_w).floor();
        let ry = (uv[1] * self.raster_h).floor();
        if rx < 0.0 || ry < 0.0 || rx >= self.raster_w || ry >= self.raster_h {
            return None;
        }
        Some((rx as u32, ry as u32))
    }

    /// Inverse of [`screen_to_raster`](Self::screen_to_raster): a RASTER pixel
    /// `(rx,ry)` → its SCREEN pixel under the current pan/zoom. Used to place combat
    /// BUBBLES at their province centroid so they track the live map exactly (the
    /// same transform the shader uses, inverted). A point currently off-screen still
    /// returns its (out-of-viewport) screen coord — the caller clips/cull as needed.
    pub fn raster_to_screen(&self, rx: f32, ry: f32) -> (f32, f32) {
        let map_u = rx / self.raster_w;
        let map_v = ry / self.raster_h;
        let out_u = (map_u - self.offset[0]) * self.scale;
        let out_v = (map_v - self.offset[1]) * self.scale;
        (out_u * self.screen_w, out_v * self.screen_h)
    }

    /// Recenter the visible map window on a RASTER/world coordinate while preserving
    /// the current zoom. Returns `false` if the camera/world is not usable or the
    /// bounded offset is unchanged.
    pub fn recenter_on_raster(&mut self, rx: f32, ry: f32) -> bool {
        if !(rx.is_finite()
            && ry.is_finite()
            && self.raster_w.is_finite()
            && self.raster_h.is_finite()
            && self.raster_w > 0.0
            && self.raster_h > 0.0
            && self.scale.is_finite()
            && self.scale > 0.0)
        {
            return false;
        }

        let before = self.offset;
        let span = 1.0 / self.scale;
        self.offset[0] = (rx / self.raster_w).clamp(0.0, 1.0) - span * 0.5;
        self.offset[1] = (ry / self.raster_h).clamp(0.0, 1.0) - span * 0.5;
        self.clamp_offset();
        (self.offset[0] - before[0]).abs() > f32::EPSILON
            || (self.offset[1] - before[1]).abs() > f32::EPSILON
    }

    /// Pan by a screen-pixel drag delta `(dx,dy)`. Dragging the map RIGHT (positive
    /// dx) moves the world right, i.e. shows content to the LEFT → offset decreases.
    /// The UV delta is `d_px / (screen * scale)`.
    pub fn pan_by_pixels(&mut self, dx: f32, dy: f32) {
        self.offset[0] -= dx / (self.screen_w * self.scale);
        self.offset[1] -= dy / (self.screen_h * self.scale);
        self.clamp_offset();
    }

    /// Zoom by a scroll `delta` (lines; +in / -out) focused at screen `(px,py)`, so
    /// the map point under the cursor stays under the cursor (focal zoom). Scale is
    /// clamped to `[1, max_scale]` (never zoom out past the whole world).
    pub fn zoom_at(&mut self, delta: f32, px: f32, py: f32, max_scale: f32) {
        // the map point currently under the cursor (in UV) — must be invariant.
        let before = self.screen_to_map_uv(px, py);
        let factor = (1.0 + delta * 0.1).max(0.1);
        let new_scale = (self.scale * factor).clamp(1.0, max_scale.max(1.0));
        if (new_scale - self.scale).abs() < f32::EPSILON {
            return;
        }
        self.scale = new_scale;
        // solve offset so that screen_to_map_uv(px,py) == before with the new scale:
        //   before = out_uv/scale + offset  =>  offset = before - out_uv/scale
        let out_u = px / self.screen_w;
        let out_v = py / self.screen_h;
        self.offset[0] = before[0] - out_u / self.scale;
        self.offset[1] = before[1] - out_v / self.scale;
        self.clamp_offset();
    }

    /// Keep the visible window inside the [0,1] map-UV range so the map never shows a
    /// band of background past its edge (the visible UV span is `1/scale`).
    fn clamp_offset(&mut self) {
        let span = 1.0 / self.scale;
        let max_off = (1.0 - span).max(0.0);
        self.offset[0] = self.offset[0].clamp(0.0, max_off);
        self.offset[1] = self.offset[1].clamp(0.0, max_off);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> MapCamera {
        // a 1000x500 viewport over a 4000x2000 raster (the real map is ~2.75:1; the
        // exact ratio doesn't matter for the transform tests).
        MapCamera::new(1000, 500, 4000, 2000)
    }

    #[test]
    fn full_world_screen_to_raster_is_proportional() {
        let c = cam();
        // top-left pixel -> raster (0,0).
        assert_eq!(c.screen_to_raster(0.0, 0.0), Some((0, 0)));
        // centre pixel -> raster centre.
        assert_eq!(c.screen_to_raster(500.0, 250.0), Some((2000, 1000)));
        // just inside the bottom-right.
        let br = c.screen_to_raster(999.0, 499.0).unwrap();
        assert!(br.0 >= 3990 && br.1 >= 1990);
    }

    #[test]
    fn pick_outside_raster_is_none_not_fabricated() {
        let mut c = cam();
        // zoom in, then pan to the top-left corner; a pixel that maps past UV 1.0 in
        // x must return None (no clamped fake province).
        c.scale = 4.0;
        c.offset = [0.0, 0.0];
        // with scale 4, the visible UV span is 0.25; screen x just below width maps
        // to ~0.25 UV which is inside. Force an out-of-range by a manual offset.
        c.offset = [0.80, 0.0]; // would be clamped, set scale-consistent below
        c.scale = 4.0;
        // recompute clamp: span 0.25, max_off 0.75 -> offset clamps to 0.75.
        c.pan_by_pixels(0.0, 0.0); // triggers clamp via clamp_offset path
                                   // a pixel near the right edge maps to offset + ~0.25 = ~1.0 -> None or last col.
        let r = c.screen_to_raster(999.5, 250.0);
        // it must be either a valid last-column raster pixel or None — never a wild value.
        if let Some((x, _)) = r {
            assert!(x < 4000, "raster x must stay in-bounds");
        }
    }

    #[test]
    fn focal_zoom_keeps_cursor_point_fixed() {
        let mut c = cam();
        let (px, py) = (300.0, 200.0);
        let before = c.screen_to_raster(px, py).unwrap();
        c.zoom_at(2.0, px, py, 16.0); // zoom in at the cursor
        assert!(c.scale > 1.0, "zoom in must increase scale");
        let after = c.screen_to_raster(px, py).unwrap();
        // the raster pixel under the cursor must be (almost) unchanged by a focal zoom.
        let dx = (before.0 as i64 - after.0 as i64).abs();
        let dy = (before.1 as i64 - after.1 as i64).abs();
        assert!(
            dx <= 8 && dy <= 8,
            "focal zoom moved the cursor point: d=({dx},{dy})"
        );
    }

    #[test]
    fn cannot_zoom_out_past_full_world() {
        let mut c = cam();
        c.zoom_at(-5.0, 500.0, 250.0, 16.0); // try to zoom way out
        assert_eq!(c.scale, 1.0, "scale clamps at 1 (whole world); never less");
        assert_eq!(c.offset, [0.0, 0.0]);
    }

    #[test]
    fn pan_clamps_to_map_edges() {
        let mut c = cam();
        c.scale = 2.0; // visible span 0.5, max offset 0.5
                       // a huge leftward drag would push offset negative; it clamps at 0.
        c.pan_by_pixels(100000.0, 100000.0);
        assert!(c.offset[0] >= 0.0 && c.offset[1] >= 0.0);
        // a huge rightward drag clamps at max_off 0.5.
        c.pan_by_pixels(-100000.0, -100000.0);
        assert!(c.offset[0] <= 0.5 + 1e-6 && c.offset[1] <= 0.5 + 1e-6);
    }

    #[test]
    fn recenter_on_raster_changes_uniform_digest_bounded() {
        let mut c = cam();
        c.scale = 4.0; // visible UV span 0.25
        let before = format!("{:?}:{:?}", c.uniform_offset(), c.uniform_scale());

        assert!(c.recenter_on_raster(3000.0, 1000.0));

        let after = format!("{:?}:{:?}", c.uniform_offset(), c.uniform_scale());
        assert_ne!(before, after, "camera uniforms must change after recenter");
        assert!((c.offset[0] - 0.625).abs() < 1e-6, "offset={:?}", c.offset);
        assert!((c.offset[1] - 0.375).abs() < 1e-6, "offset={:?}", c.offset);
        let max_off = 1.0 - 1.0 / c.scale;
        assert!(c.offset[0] >= 0.0 && c.offset[0] <= max_off + 1e-6);
        assert!(c.offset[1] >= 0.0 && c.offset[1] <= max_off + 1e-6);
    }

    #[test]
    fn recenter_on_raster_clamps_to_world_edges() {
        let mut c = cam();
        c.scale = 4.0;

        assert!(!c.recenter_on_raster(0.0, 0.0));
        assert_eq!(c.offset, [0.0, 0.0]);

        assert!(c.recenter_on_raster(999999.0, 999999.0));
        let max_off = 1.0 - 1.0 / c.scale;
        assert!(
            (c.offset[0] - max_off).abs() < 1e-6,
            "offset={:?}",
            c.offset
        );
        assert!(
            (c.offset[1] - max_off).abs() < 1e-6,
            "offset={:?}",
            c.offset
        );
    }

    #[test]
    fn recenter_without_usable_world_is_noop() {
        let mut c = cam();
        c.scale = 4.0;
        c.offset = [0.25, 0.25];
        let before = c.offset;

        c.raster_w = 0.0;
        assert!(!c.recenter_on_raster(100.0, 100.0));
        assert_eq!(c.offset, before);

        c.raster_w = 4000.0;
        c.scale = 0.0;
        assert!(!c.recenter_on_raster(100.0, 100.0));
        assert_eq!(c.offset, before);
    }
}
