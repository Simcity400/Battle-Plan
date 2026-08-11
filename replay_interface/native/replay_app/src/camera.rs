//! Viewer-owned product camera.
//!
//! The public center/zoom projection is retained for the existing renderer
//! boundary, but production input mutates the authenticated XMAP03 3D state.
//! A legacy 2D instance remains only for the explicit debug bundle/dump route.

use map_viewer::ViewerMapCameraState;

use crate::Result;

#[derive(Clone, Debug)]
pub struct Camera {
    pub center_x: f64,
    pub center_y: f64,
    /// Screen pixels per map pixel.
    pub zoom: f64,
    map_w: f64,
    map_h: f64,
    viewer_state: Option<ViewerMapCameraState>,
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
            viewer_state: None,
        };
        camera
            .reset_fit(screen_w, screen_h)
            .expect("legacy debug camera reset");
        camera
    }

    pub fn from_viewer_state(state: ViewerMapCameraState) -> Result<Self> {
        state.validate()?;
        let mut camera = Self {
            center_x: 0.0,
            center_y: 0.0,
            zoom: 1.0,
            map_w: f64::from(state.map_width),
            map_h: f64::from(state.map_height),
            viewer_state: Some(state),
        };
        camera.sync_projection()?;
        Ok(camera)
    }

    pub fn viewer_state(&self) -> Option<&ViewerMapCameraState> {
        self.viewer_state.as_ref()
    }

    /// Reset to whole-map fit for the given screen size.
    pub fn reset_fit(&mut self, screen_w: f64, screen_h: f64) -> Result<()> {
        if let Some(state) = &mut self.viewer_state {
            let width = screen_w.max(1.0).round() as u32;
            let height = screen_h.max(1.0).round() as u32;
            state.resize_viewport(width, height)?;
            state.reset_to_engine_overview()?;
            self.sync_projection()?;
            return Ok(());
        }
        let screen_w = screen_w.max(1.0);
        let screen_h = screen_h.max(1.0);
        self.zoom = (screen_w / self.map_w).min(screen_h / self.map_h);
        self.center_x = self.map_w * 0.5;
        self.center_y = self.map_h * 0.5;
        self.normalize(screen_h);
        Ok(())
    }

    /// Screen position -> continuous (unwrapped) map position.
    pub fn screen_to_map(&self, sx: f64, sy: f64, screen_w: f64, screen_h: f64) -> (f64, f64) {
        if let Some(state) = &self.viewer_state {
            return state
                .screen_to_map(sx as f32, sy as f32)
                .expect("validated viewer screen-to-map")
                .map(|(x, y)| (f64::from(x), f64::from(y)))
                .unwrap_or((f64::NAN, f64::NAN));
        }
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

    /// Pan by a screen-pixel delta through the viewer-owned 3D ground
    /// projection. Positive x moves east and positive y moves south.
    pub fn pan_screen(&mut self, dx: f64, dy: f64, screen_h: f64) -> Result<()> {
        if let Some(state) = &mut self.viewer_state {
            state.pan_screen(dx as f32, dy as f32)?;
            self.sync_projection()?;
            return Ok(());
        }
        self.center_x += dx / self.zoom;
        self.center_y += dy / self.zoom;
        self.normalize(screen_h);
        Ok(())
    }

    pub fn drag_from(&mut self, start: &Camera, dx: f64, dy: f64, screen_h: f64) -> Result<()> {
        *self = start.clone();
        self.pan_screen(-dx, -dy, screen_h)
    }

    pub fn wheel_zoom(
        &mut self,
        cursor_x: f64,
        cursor_y: f64,
        signed_delta: i32,
        raw_owner_delta: f64,
    ) -> Result<()> {
        let state = self
            .viewer_state
            .as_mut()
            .ok_or("wheel zoom is unavailable on the legacy debug camera")?;
        state.wheel_zoom(
            cursor_x as f32,
            cursor_y as f32,
            signed_delta,
            raw_owner_delta as f32,
        )?;
        self.sync_projection()
    }

    pub fn advance(&mut self, raw_owner_delta: f64) -> Result<()> {
        if let Some(state) = &mut self.viewer_state {
            state.advance(raw_owner_delta as f32)?;
            self.sync_projection()?;
        }
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if let Some(state) = &mut self.viewer_state {
            state.resize_viewport(width.max(1), height.max(1))?;
            self.sync_projection()?;
        } else {
            self.normalize(f64::from(height.max(1)));
        }
        Ok(())
    }

    /// Wrap the horizontal center. Vertical center clamping is not performed:
    /// C03 refutes a center clamp and requires projected ground intersections.
    pub fn normalize(&mut self, _screen_h: f64) {
        if self.viewer_state.is_some() {
            self.sync_projection()
                .expect("validated viewer camera projection");
            return;
        }
        self.center_x = self.center_x.rem_euclid(self.map_w);
    }

    fn sync_projection(&mut self) -> Result<()> {
        let Some(state) = &self.viewer_state else {
            return Ok(());
        };
        let (center_x, center_y) = state.center_map_pixels();
        self.center_x = center_x;
        self.center_y = center_y;
        self.zoom = state.screen_zoom()?;
        Ok(())
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
        camera.pan_screen(-100.0, 0.0, SCREEN_H).unwrap();
        assert!(camera.center_x >= 0.0 && camera.center_x < 5632.0);
        assert_close(camera.center_x, 5632.0 + 10.0 - 50.0, "wrapped center");
    }
}
