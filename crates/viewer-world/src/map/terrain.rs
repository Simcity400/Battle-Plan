//! The approved uniform terrain grid supplied to the compiled content shader.

use viewer_foundation::map::camera::Camera;
#[cfg(test)]
use viewer_foundation::map::camera::MapDefines;
#[cfg(test)]
use viewer_foundation::map::camera::MapSize;

/// Quads the visible grid may hold before the step doubles.
///
/// At the reference camera one map pixel is 5.73 screen pixels and the visible
/// ground is about 464 x 252 units, so a step of 1 costs about 140k quads with
/// the margin. The ceiling only bites when the camera climbs:
/// at `CAMERA_MAX_HEIGHT` the visible ground is roughly 4400 x 2475 units,
/// which a step of 1 would turn into 10.9M quads.
pub const MAX_QUADS: u32 = 600_000;

/// World units the visible rectangle grows by on every side.
///
/// Terrain reaches 25.5 units above the water plane, and at the shallowest
/// pitch the engine allows (53.5 degrees below the horizon at height 50) a
/// 16-unit column leans about 12 units past its own ground position, so 16
/// units of margin keeps everything that can appear inside the frame.
pub const MARGIN: f32 = 16.0;

pub use super::terrain_shader::Grid;

/// Covers `rect` with the finest power-of-two grid that stays within `max_quads`.
///
/// `rect` is `(x0, z0, x1, z1)` in world units. The `z` range is clipped to the
/// map, which has no terrain outside it; `x` is left alone because the map wraps
/// and the shaders wrap their own lookups.
pub fn grid(rect: [f32; 4], map_height: f32, max_quads: u32) -> Grid {
    let z0 = rect[1].max(-MARGIN);
    let z1 = rect[3].min(map_height - 1.0 + MARGIN).max(z0);
    let mut step = 1.0f32;
    loop {
        let origin = [(rect[0] / step).floor() * step, (z0 / step).floor() * step];
        let counts = [
            ((rect[2] - origin[0]) / step).ceil().max(1.0) as u32 + 1,
            ((z1 - origin[1]) / step).ceil().max(1.0) as u32 + 1,
        ];
        let candidate = Grid {
            origin,
            step,
            counts,
        };
        if candidate.quads() <= max_quads || step >= 256.0 {
            return candidate;
        }
        step *= 2.0;
    }
}

/// The grid for one camera and viewport.
pub fn grid_for(camera: Camera, viewport: [u32; 2]) -> Grid {
    match camera.visible_ground(viewport, MARGIN) {
        Some(rect) => grid(rect, camera.map.y(), MAX_QUADS),
        // A camera that cannot see the water plane cannot see the map either.
        None => Grid {
            origin: [0.0, 0.0],
            step: 1.0,
            counts: [0, 0],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: f32 = 2048.0;

    /// World `(x, z)` of the last vertex the grid places.
    fn far_corner(grid: Grid) -> [f32; 2] {
        [
            grid.origin[0] + (grid.counts[0] - 1) as f32 * grid.step,
            grid.origin[1] + (grid.counts[1] - 1) as f32 * grid.step,
        ]
    }

    #[test]
    fn the_grid_covers_the_rectangle_at_step_one_when_it_is_small() {
        let grid = grid([100.4, 200.9, 140.0, 220.0], H, MAX_QUADS);
        assert_eq!(grid.step, 1.0);
        assert_eq!(grid.origin, [100.0, 200.0]);
        // 100..140 needs 41 columns, 200..220 needs 21 rows.
        assert_eq!(grid.counts, [41, 21]);
        let far = far_corner(grid);
        assert!(far[0] >= 140.0 && far[1] >= 220.0);
        assert_eq!(grid.quads(), 40 * 20);
    }

    #[test]
    fn the_step_doubles_until_the_quad_budget_holds() {
        // The full-map view at CAMERA_MAX_HEIGHT: about 4400 x 2475 units,
        // of which only the map's own 2048 rows plus the margin survive.
        let rect = [0.0, 0.0, 4400.0, 2475.0];
        let coarse = grid(rect, H, MAX_QUADS);
        assert!(coarse.quads() <= MAX_QUADS, "{coarse:?}");
        // Step 2 would be 2.27M quads and step 4 is 567_600, just inside.
        assert_eq!(coarse.step, 4.0);
        assert_eq!(coarse.quads(), 1100 * 516);
        // A four-times budget admits step 2, which is what makes the rule a
        // doubling against the budget and not a fixed choice.
        assert_eq!(grid(rect, H, MAX_QUADS * 4).step, 2.0);
        // The step never runs away, even on an impossible budget.
        assert_eq!(grid(rect, H, 1).step, 256.0);
    }

    #[test]
    fn the_origin_stays_on_a_multiple_of_the_step() {
        let grid = grid([-13.0, 7.0, 4400.0, 2475.0], H, MAX_QUADS);
        assert_eq!(grid.origin[0] % grid.step, 0.0);
        assert_eq!(grid.origin[1] % grid.step, 0.0);
        assert!(grid.origin[0] <= -13.0 && grid.origin[1] <= 7.0);
    }

    #[test]
    fn the_grid_is_clipped_to_the_map_in_z_but_not_in_x() {
        let grid = grid([-400.0, -900.0, 200.0, 9000.0], H, MAX_QUADS);
        assert!(grid.origin[0] < 0.0, "x is never clipped: the map wraps");
        assert!(grid.origin[1] >= -MARGIN - grid.step);
        let top = far_corner(grid)[1];
        assert!(top <= H - 1.0 + MARGIN + grid.step, "{top}");
    }

    #[test]
    fn the_reference_camera_grid_is_about_a_hundred_thousand_quads() {
        let camera = Camera {
            center: [3010.0, 1568.0],
            height: 310.178,
            map: MapSize::TEST,
            defines: MapDefines::test(),
        };
        let grid = grid_for(camera, [2048, 1152]);
        assert_eq!(grid.step, 1.0);
        assert!(
            (100_000..250_000).contains(&grid.quads()),
            "{} quads",
            grid.quads()
        );
    }
}
