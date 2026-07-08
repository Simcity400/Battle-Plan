//! Map camera. Coordinates live in map pixels per the Phase 2 architecture
//! of `MAP_REPLICA_PLAN.md`: `center_x`/`center_y` are the map pixel at the
//! screen center and `zoom` is screen pixels per map pixel. The the game map
//! wraps east-west, so x wraps and y clamps. Pure math, no D3D.

// Interaction tuning constants (not visual claims): absolute zoom bounds
// for the mouse-wheel/scroll range.
const MIN_ZOOM: f64 = 0.01;
const MAX_ZOOM: f64 = 256.0;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub center_x: f64,
    pub center_y: f64,
    /// Screen pixels per map pixel.
    pub zoom: f64,
    map_w: f64,
    map_h: f64,
}

impl Camera {
    /// Camera fitted so the whole map is visible in the given screen.
    pub fn fit(map_w: usize, map_h: usize, screen_w: f64, screen_h: f64) -> Camera {
        let map_w = (map_w.max(1)) as f64;
        let map_h = (map_h.max(1)) as f64;
        let mut camera = Camera {
            center_x: map_w * 0.5,
            center_y: map_h * 0.5,
            zoom: 1.0,
            map_w,
            map_h,
        };
        camera.reset_fit(screen_w, screen_h);
        camera
    }

    /// Reset to whole-map fit for the given screen size.
    pub fn reset_fit(&mut self, screen_w: f64, screen_h: f64) {
        let screen_w = screen_w.max(1.0);
        let screen_h = screen_h.max(1.0);
        self.zoom = (screen_w / self.map_w).min(screen_h / self.map_h);
        self.center_x = self.map_w * 0.5;
        self.center_y = self.map_h * 0.5;
        self.normalize(screen_h);
    }

    /// Screen position -> continuous (unwrapped) map position.
    pub fn screen_to_map(&self, sx: f64, sy: f64, screen_w: f64, screen_h: f64) -> (f64, f64) {
        (
            self.center_x + (sx - screen_w * 0.5) / self.zoom,
            self.center_y + (sy - screen_h * 0.5) / self.zoom,
        )
    }

    /// Continuous map position -> screen position (inverse of
    /// [`Camera::screen_to_map`]; does not itself apply wrapping). Used
    /// by the transform round-trip tests; overlays will use it next.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn map_to_screen(&self, mx: f64, my: f64, screen_w: f64, screen_h: f64) -> (f64, f64) {
        (
            (mx - self.center_x) * self.zoom + screen_w * 0.5,
            (my - self.center_y) * self.zoom + screen_h * 0.5,
        )
    }

    /// Wrap a continuous map x into `[0, map_w)` (east-west wrap).
    pub fn wrap_x(&self, x: f64) -> f64 {
        x.rem_euclid(self.map_w)
    }

    /// Map height in map pixels (needed to flip between this camera's
    /// bmp-style y-down coordinates and the renderer's world z-up).
    pub fn map_h(&self) -> f64 {
        self.map_h
    }

    /// Pan by a screen-pixel delta (positive dx moves the view east).
    pub fn pan_screen(&mut self, dx: f64, dy: f64, screen_h: f64) {
        self.center_x += dx / self.zoom;
        self.center_y += dy / self.zoom;
        self.normalize(screen_h);
    }

    /// Zoom by `factor` keeping the map point under the cursor fixed.
    pub fn zoom_at(
        &mut self,
        cursor_x: f64,
        cursor_y: f64,
        screen_w: f64,
        screen_h: f64,
        factor: f64,
    ) {
        let (anchor_x, anchor_y) = self.screen_to_map(cursor_x, cursor_y, screen_w, screen_h);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.center_x = anchor_x - (cursor_x - screen_w * 0.5) / self.zoom;
        self.center_y = anchor_y - (cursor_y - screen_h * 0.5) / self.zoom;
        self.normalize(screen_h);
    }

    /// Wrap the horizontal center and clamp the vertical center so the
    /// visible window stays on the map (or centers when zoomed out past
    /// full map height).
    pub fn normalize(&mut self, screen_h: f64) {
        self.center_x = self.center_x.rem_euclid(self.map_w);
        let visible_h = screen_h.max(1.0) / self.zoom;
        if visible_h >= self.map_h {
            self.center_y = self.map_h * 0.5;
        } else {
            let half = visible_h * 0.5;
            self.center_y = self.center_y.clamp(half, self.map_h - half);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN_W: f64 = 1280.0;
    const SCREEN_H: f64 = 720.0;

    fn assert_close(a: f64, b: f64, message: &str) {
        assert!((a - b).abs() < 1e-9, "{message}: {a} vs {b}");
    }

    #[test]
    fn fit_shows_whole_map_centered() {
        let camera = Camera::fit(5632, 2048, SCREEN_W, SCREEN_H);
        assert_close(camera.center_x, 2816.0, "fit center x");
        assert_close(camera.center_y, 1024.0, "fit center y");
        let expected_zoom = (SCREEN_W / 5632.0).min(SCREEN_H / 2048.0);
        assert_close(camera.zoom, expected_zoom, "fit zoom");
        // The whole map must fit inside the screen at fit zoom.
        assert!(5632.0 * camera.zoom <= SCREEN_W + 1e-9);
        assert!(2048.0 * camera.zoom <= SCREEN_H + 1e-9);
    }

    #[test]
    fn screen_map_round_trip() {
        let mut camera = Camera::fit(5632, 2048, SCREEN_W, SCREEN_H);
        camera.zoom = 3.5;
        camera.center_x = 1234.25;
        camera.center_y = 640.75;
        for &(sx, sy) in &[(0.5, 0.5), (640.0, 360.0), (1279.5, 719.5), (17.0, 703.0)] {
            let (mx, my) = camera.screen_to_map(sx, sy, SCREEN_W, SCREEN_H);
            let (rx, ry) = camera.map_to_screen(mx, my, SCREEN_W, SCREEN_H);
            assert_close(rx, sx, "round trip x");
            assert_close(ry, sy, "round trip y");
        }
    }

    #[test]
    fn wrap_x_wraps_east_west() {
        let camera = Camera::fit(5632, 2048, SCREEN_W, SCREEN_H);
        assert_close(camera.wrap_x(-1.0), 5631.0, "west wrap");
        assert_close(camera.wrap_x(5632.0), 0.0, "east wrap");
        assert_close(camera.wrap_x(5633.5), 1.5, "east overflow");
        assert_close(camera.wrap_x(100.0), 100.0, "in range");
    }

    #[test]
    fn pan_across_seam_wraps_center() {
        let mut camera = Camera::fit(5632, 2048, SCREEN_W, SCREEN_H);
        camera.zoom = 2.0;
        camera.center_x = 10.0;
        // Pan far enough west that the unwrapped center goes negative.
        camera.pan_screen(-100.0, 0.0, SCREEN_H);
        assert!(camera.center_x >= 0.0 && camera.center_x < 5632.0);
        assert_close(camera.center_x, 5632.0 + 10.0 - 50.0, "wrapped center");
    }

    #[test]
    fn zoom_at_cursor_keeps_map_point_fixed() {
        let mut camera = Camera::fit(5632, 2048, SCREEN_W, SCREEN_H);
        camera.zoom = 2.0;
        camera.center_x = 2816.0;
        camera.center_y = 1024.0;
        let cursor = (321.0, 654.0);
        let before = camera.screen_to_map(cursor.0, cursor.1, SCREEN_W, SCREEN_H);
        camera.zoom_at(cursor.0, cursor.1, SCREEN_W, SCREEN_H, 1.25);
        let after = camera.screen_to_map(cursor.0, cursor.1, SCREEN_W, SCREEN_H);
        assert_close(after.0, before.0, "anchored map x");
        assert_close(after.1, before.1, "anchored map y");
        assert_close(camera.zoom, 2.5, "zoom factor applied");
    }

    #[test]
    fn vertical_center_clamps_to_map() {
        let mut camera = Camera::fit(5632, 2048, SCREEN_W, SCREEN_H);
        camera.zoom = 2.0; // visible height 360 map px, half = 180
        camera.center_y = -500.0;
        camera.normalize(SCREEN_H);
        assert_close(camera.center_y, 180.0, "top clamp");
        camera.center_y = 99999.0;
        camera.normalize(SCREEN_H);
        assert_close(camera.center_y, 2048.0 - 180.0, "bottom clamp");
    }

    #[test]
    fn zoomed_out_centers_vertically() {
        let mut camera = Camera::fit(5632, 2048, SCREEN_W, SCREEN_H);
        camera.zoom = 0.05; // visible height far exceeds map height
        camera.center_y = 3.0;
        camera.normalize(SCREEN_H);
        assert_close(camera.center_y, 1024.0, "letterbox centers map");
    }
}
