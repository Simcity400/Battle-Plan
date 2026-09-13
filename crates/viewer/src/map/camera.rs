//! The game's map camera, ported exactly from research R1 sections 1.1 to 1.3.
//!
//! The engine keeps a left-handed look-at camera with a fixed horizontal lead:
//! `eye = (cx, h, cz - 30)` looking at `(cx, WATER_HEIGHT, cz)` with up `+Y`.
//! The 30 is `camera_tilt (100) * 0.3` and does not depend on the height, so
//! "pitch" is really a constant lead and `height` is the only zoom control.
//!
//! Matrices follow D3D's row-vector convention (`clip = vec4(world, 1) * M`),
//! which is what [`view_projection`](Camera::view_projection) returns as
//! `[row][column]`. WGSL reads a `mat4x4<f32>` uniform as four consecutive
//! column vectors, so writing those rows straight into the buffer gives WGSL
//! the transpose, which is exactly the column-vector matrix `M^T` that yields
//! the same clip position from `M_wgsl * vec4(world, 1)`. Depth lands in 0..1
//! in both D3D and wgpu, so no depth remap is needed.

/// Vertical field of view in radians (`0x142953540`, R1 section 1.1).
pub const FOV_Y: f32 = 0.785;
/// Near plane (`0x14270e55c`).
pub const NEAR: f32 = 5.0;
/// Far plane (`0x142934a8c`).
pub const FAR: f32 = 4000.0;
/// `WATER_HEIGHT` from `constants.fxh`; the look-at point and water plane.
pub const WATER_HEIGHT: f32 = 9.5;
/// `camera_tilt (100) * 0.3`: the look-at point leads the eye northwards.
pub const TILT_LEAD: f32 = 30.0;
/// `CAMERA_MIN_HEIGHT` (`00_graphics.lua:1495`).
pub const MIN_HEIGHT: f32 = 50.0;
/// `CAMERA_MAX_HEIGHT` (`00_graphics.lua:1496`).
pub const MAX_HEIGHT: f32 = 3000.0;
/// `MAP_SIZE_X` injected into every map shader (R1 section 3.5).
pub const MAP_SIZE_X: f32 = 5632.0;
/// `MAP_SIZE_Y` injected into every map shader.
pub const MAP_SIZE_Y: f32 = 2048.0;

/// The map camera: the ground point it looks at, and its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The look-at point on the water plane, as world `(x, z)`.
    pub center: [f32; 2],
    /// Eye height in world units; the only zoom control the engine has.
    pub height: f32,
}

/// An orthonormal left-handed camera basis: right, up and forward.
struct Basis {
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
}

impl Camera {
    /// The eye position, `vCamPos` in the frame constants.
    pub fn eye(&self) -> [f32; 3] {
        [self.center[0], self.height, self.center[1] - TILT_LEAD]
    }

    /// The look-at point, fixed to the water plane.
    pub fn look_at(&self) -> [f32; 3] {
        [self.center[0], WATER_HEIGHT, self.center[1]]
    }

    /// The normalised forward direction, `vCamLookAtDir`.
    pub fn look_direction(&self) -> [f32; 3] {
        normalize(subtract(self.look_at(), self.eye()))
    }

    fn basis(&self) -> Basis {
        let forward = self.look_direction();
        // LookAtLH: right = normalize(cross(up, forward)), up = cross(forward, right).
        let right = normalize(cross([0.0, 1.0, 0.0], forward));
        Basis {
            up: cross(forward, right),
            right,
            forward,
        }
    }

    /// `ViewProjectionMatrix` as `[row][column]` in the row-vector convention.
    pub fn view_projection(&self, viewport: [u32; 2]) -> [[f32; 4]; 4] {
        multiply(self.view(), projection(aspect(viewport)))
    }

    /// `InvViewProjMatrix`, in the same convention; `None` if it is singular.
    pub fn inverse(&self, viewport: [u32; 2]) -> Option<[[f32; 4]; 4]> {
        invert(self.view_projection(viewport))
    }

    fn view(&self) -> [[f32; 4]; 4] {
        let Basis { right, up, forward } = self.basis();
        let eye = self.eye();
        [
            [right[0], up[0], forward[0], 0.0],
            [right[1], up[1], forward[1], 0.0],
            [right[2], up[2], forward[2], 0.0],
            [-dot(right, eye), -dot(up, eye), -dot(forward, eye), 1.0],
        ]
    }

    /// Projects a world point to screen pixels, with `y` down from the top.
    ///
    /// `None` when the point is at or behind the eye plane, where the
    /// perspective divide has no meaning.
    // The `names` and `icons` passes anchor their screen-space sprites with
    // this (R1 "Map-icon anchoring"); no pass in this slice draws in 2D yet.
    #[allow(dead_code)]
    pub fn project(&self, world: [f32; 3], viewport: [u32; 2]) -> Option<[f32; 2]> {
        let m = self.view_projection(viewport);
        let mut clip = [0.0f32; 4];
        for (column, value) in clip.iter_mut().enumerate() {
            *value = world[0] * m[0][column]
                + world[1] * m[1][column]
                + world[2] * m[2][column]
                + m[3][column];
        }
        if !clip[3].is_finite() || clip[3] <= 1e-6 {
            return None;
        }
        Some([
            (clip[0] / clip[3] * 0.5 + 0.5) * viewport[0].max(1) as f32,
            (0.5 - clip[1] / clip[3] * 0.5) * viewport[1].max(1) as f32,
        ])
    }

    /// Where a screen pixel's view ray meets the water plane, as world `(x, z)`.
    ///
    /// `None` when the ray points at or above the horizon. At every height the
    /// engine allows this cannot happen: the steepest lead is
    /// `atan2(30, 50 - 9.5) = 36.5` degrees above the forward axis while half
    /// the vertical field of view is only 22.5 degrees (R1 section 1.2).
    pub fn unproject_ground(&self, screen: [f32; 2], viewport: [u32; 2]) -> Option<[f32; 2]> {
        let direction = self.ray(screen, viewport);
        if direction[1] > -1e-6 {
            return None;
        }
        let eye = self.eye();
        let t = (WATER_HEIGHT - eye[1]) / direction[1];
        Some([eye[0] + t * direction[0], eye[2] + t * direction[2]])
    }

    /// The unnormalised world-space view ray through a screen pixel.
    fn ray(&self, screen: [f32; 2], viewport: [u32; 2]) -> [f32; 3] {
        let width = viewport[0].max(1) as f32;
        let height = viewport[1].max(1) as f32;
        let ndc_x = screen[0] / width * 2.0 - 1.0;
        let ndc_y = 1.0 - screen[1] / height * 2.0;
        let y_scale = 1.0 / (FOV_Y * 0.5).tan();
        // View-space direction that projects onto this NDC point, with z = 1.
        let view = [ndc_x * aspect(viewport) / y_scale, ndc_y / y_scale, 1.0];
        let Basis { right, up, forward } = self.basis();
        [
            right[0] * view[0] + up[0] * view[1] + forward[0] * view[2],
            right[1] * view[0] + up[1] * view[1] + forward[1] * view[2],
            right[2] * view[0] + up[2] * view[1] + forward[2] * view[2],
        ]
    }

    /// Drags the ground point under `from` to sit under `to`.
    ///
    /// The camera only ever translates in `xz`, so the ground point under a
    /// screen pixel is `center + f(pixel)` for a function of the height and the
    /// viewport alone; the new centre follows without another intersection.
    pub fn pan(&mut self, from: [f32; 2], to: [f32; 2], viewport: [u32; 2]) {
        let (Some(from), Some(to)) = (
            self.unproject_ground(from, viewport),
            self.unproject_ground(to, viewport),
        ) else {
            return;
        };
        self.center[0] += from[0] - to[0];
        self.center[1] += from[1] - to[1];
    }

    /// Changes the height by `factor`, keeping the ground point under `screen`.
    pub fn zoom(&mut self, factor: f32, screen: [f32; 2], viewport: [u32; 2]) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let Some(before) = self.unproject_ground(screen, viewport) else {
            return;
        };
        self.height = (self.height / factor).clamp(MIN_HEIGHT, MAX_HEIGHT);
        let Some(after) = self.unproject_ground(screen, viewport) else {
            return;
        };
        self.center[0] += before[0] - after[0];
        self.center[1] += before[1] - after[1];
    }

    /// Centres the map and picks the lowest height that shows all of it.
    ///
    /// The visible ground quad is not a linear function of the height, because
    /// the 30-unit lead is fixed while the eye rises, so this bisects instead of
    /// solving. The engine's `CAMERA_MAX_HEIGHT` of 3000 does not reach the full
    /// 5632-wide map at 16:9, so the result saturates there.
    pub fn fit(map: [f32; 2], viewport: [u32; 2]) -> Self {
        let center = [map[0] * 0.5, map[1] * 0.5];
        let covers = |height: f32| {
            let camera = Camera { center, height };
            match camera.visible_ground(viewport, 0.0) {
                Some(rect) => {
                    rect[0] <= 0.0 && rect[1] <= 0.0 && rect[2] >= map[0] && rect[3] >= map[1]
                }
                None => false,
            }
        };
        let mut low = MIN_HEIGHT;
        let mut high = MAX_HEIGHT;
        if !covers(high) {
            return Self {
                center,
                height: MAX_HEIGHT,
            };
        }
        for _ in 0..40 {
            let middle = 0.5 * (low + high);
            if covers(middle) {
                high = middle;
            } else {
                low = middle;
            }
        }
        Self {
            center,
            height: high,
        }
    }

    /// The `(x0, z0, x1, z1)` bounding box of the visible water plane, grown by
    /// `margin` on every side so terrain standing above the plane stays covered.
    pub fn visible_ground(&self, viewport: [u32; 2], margin: f32) -> Option<[f32; 4]> {
        let width = viewport[0].max(1) as f32;
        let height = viewport[1].max(1) as f32;
        let mut rect = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for corner in [[0.0, 0.0], [width, 0.0], [0.0, height], [width, height]] {
            let point = self.unproject_ground(corner, viewport)?;
            rect[0] = rect[0].min(point[0]);
            rect[1] = rect[1].min(point[1]);
            rect[2] = rect[2].max(point[0]);
            rect[3] = rect[3].max(point[1]);
        }
        if !rect.iter().all(|value| value.is_finite()) {
            return None;
        }
        Some([
            rect[0] - margin,
            rect[1] - margin,
            rect[2] + margin,
            rect[3] + margin,
        ])
    }

    /// Refuses a camera that would produce a degenerate matrix.
    pub fn validate(self) -> std::io::Result<()> {
        if self.center.iter().all(|value| value.is_finite())
            && self.height.is_finite()
            && self.height > WATER_HEIGHT
        {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "camera centre must be finite and height must exceed the water plane {WATER_HEIGHT}"
                ),
            ))
        }
    }
}

fn aspect(viewport: [u32; 2]) -> f32 {
    viewport[0].max(1) as f32 / viewport[1].max(1) as f32
}

/// `D3DXMatrixPerspectiveFovLH` with `z_ndc` in 0..1 (R1 section 1.1).
fn projection(aspect: f32) -> [[f32; 4]; 4] {
    let y_scale = 1.0 / (FOV_Y * 0.5).tan();
    [
        [y_scale / aspect, 0.0, 0.0, 0.0],
        [0.0, y_scale, 0.0, 0.0],
        [0.0, 0.0, FAR / (FAR - NEAR), 1.0],
        [0.0, 0.0, -NEAR * FAR / (FAR - NEAR), 0.0],
    ]
}

fn multiply(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for (row, values) in out.iter_mut().enumerate() {
        for (column, value) in values.iter_mut().enumerate() {
            *value = (0..4).map(|k| a[row][k] * b[k][column]).sum();
        }
    }
    out
}

/// Cofactor inverse, computed in `f64` because the view-projection mixes
/// world coordinates in the thousands with near-unit projection terms.
/// `None` when the determinant is not usable.
fn invert(m: [[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
    let wide = m.map(|row| row.map(f64::from));
    let f = |r: usize, c: usize| wide[r][c];
    let mut cofactor = [[0.0f64; 4]; 4];
    for (row, cofactor_row) in cofactor.iter_mut().enumerate() {
        for (column, cofactor_value) in cofactor_row.iter_mut().enumerate() {
            let rows: Vec<usize> = (0..4).filter(|&r| r != row).collect();
            let columns: Vec<usize> = (0..4).filter(|&c| c != column).collect();
            let minor = f(rows[0], columns[0])
                * (f(rows[1], columns[1]) * f(rows[2], columns[2])
                    - f(rows[1], columns[2]) * f(rows[2], columns[1]))
                - f(rows[0], columns[1])
                    * (f(rows[1], columns[0]) * f(rows[2], columns[2])
                        - f(rows[1], columns[2]) * f(rows[2], columns[0]))
                + f(rows[0], columns[2])
                    * (f(rows[1], columns[0]) * f(rows[2], columns[1])
                        - f(rows[1], columns[1]) * f(rows[2], columns[0]));
            let sign = if (row + column) % 2 == 0 { 1.0 } else { -1.0 };
            *cofactor_value = sign * minor;
        }
    }
    let determinant: f64 = (0..4).map(|c| wide[0][c] * cofactor[0][c]).sum();
    if !determinant.is_finite() || determinant.abs() < 1e-30 {
        return None;
    }
    let mut out = [[0.0f32; 4]; 4];
    for (row, values) in out.iter_mut().enumerate() {
        for (column, value) in values.iter_mut().enumerate() {
            // Transposed cofactor matrix over the determinant.
            *value = (cofactor[column][row] / determinant) as f32;
        }
    }
    Some(out)
}

fn subtract(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    if length > 0.0 {
        [v[0] / length, v[1] / length, v[2] / length]
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE: Camera = Camera {
        center: [3010.0, 1568.0],
        height: 310.178,
    };
    const VIEWPORT: [u32; 2] = [2048, 1152];

    /// R1 section 1.5 lists measured icon centres in the 2560x1440 screenshot,
    /// which is the 2048x1152 frame upscaled by 1.25 (R1 section 0). The world
    /// anchors are `map/unitstacks.txt` type 38 positions.
    const FITTED_CITIES: [([f32; 3], [f32; 2]); 6] = [
        ([2956.37, 10.35, 1569.00], [982.1, 711.9]),
        ([3068.02, 10.40, 1541.38], [1629.1, 871.9]),
        ([3129.02, 9.82, 1561.00], [1979.1, 757.9]),
        ([3000.72, 9.70, 1638.29], [1238.1, 324.9]),
        ([2884.07, 9.70, 1564.86], [564.1, 735.9]),
        ([3103.20, 9.70, 1464.00], [1847.1, 1335.9]),
    ];

    /// R1 section 1.5 fixes the icon centre at `anchor + (9.5, -1.5)` GUI pixels.
    const ICON_OFFSET: [f32; 2] = [9.5, -1.5];

    #[test]
    fn projection_reproduces_the_fitted_city_positions() {
        for (world, measured) in FITTED_CITIES {
            let screen = REFERENCE.project(world, VIEWPORT).unwrap();
            let icon = [screen[0] + ICON_OFFSET[0], screen[1] + ICON_OFFSET[1]];
            // The report measures in the 1.25x screenshot; compare there.
            let residual = [icon[0] * 1.25 - measured[0], icon[1] * 1.25 - measured[1]];
            assert!(
                residual[0].abs() < 2.0 && residual[1].abs() < 2.0,
                "{world:?} projected to {icon:?}, residual {residual:?}"
            );
        }
    }

    #[test]
    fn the_reference_view_matrix_matches_the_report() {
        let view = REFERENCE.view();
        let expected = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.099282, -0.995059, 0.0],
            [0.0, 0.995059, 0.099282, 0.0],
            [-3010.0, -1561.1963, 155.95049, 1.0],
        ];
        for (row, values) in expected.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                assert!(
                    (view[row][column] - value).abs() < 1e-3,
                    "view[{row}][{column}] = {} not {value}",
                    view[row][column]
                );
            }
        }
        let projection = projection(aspect(VIEWPORT));
        assert!((projection[0][0] - 1.35876).abs() < 1e-5);
        assert!((projection[1][1] - 2.415574).abs() < 1e-5);
        assert!((projection[2][2] - 1.001252).abs() < 1e-5);
        assert!((projection[3][2] - -5.006258).abs() < 1e-5);
    }

    #[test]
    fn unprojecting_a_projected_point_returns_it() {
        for point in [[2900.0, 9.5, 1600.0], [3100.0, 9.5, 1500.0]] {
            let screen = REFERENCE.project(point, VIEWPORT).unwrap();
            let ground = REFERENCE.unproject_ground(screen, VIEWPORT).unwrap();
            assert!((ground[0] - point[0]).abs() < 0.01, "{ground:?}");
            assert!((ground[1] - point[2]).abs() < 0.01, "{ground:?}");
        }
    }

    #[test]
    fn the_inverse_undoes_the_view_projection() {
        let m = REFERENCE.view_projection(VIEWPORT);
        let inverse = REFERENCE.inverse(VIEWPORT).unwrap();
        let identity = multiply(m, inverse);
        for row in 0..4 {
            for column in 0..4 {
                let expected = if row == column { 1.0 } else { 0.0 };
                // The round trip is checked in f32, where the translation row
                // carries about four thousand units of magnitude.
                assert!(
                    (identity[row][column] - expected).abs() < 1e-2,
                    "{identity:?}"
                );
            }
        }
    }

    #[test]
    fn panning_keeps_the_dragged_ground_point_under_the_pointer() {
        let mut camera = REFERENCE;
        let from = [700.0, 400.0];
        let to = [900.0, 620.0];
        let grabbed = camera.unproject_ground(from, VIEWPORT).unwrap();
        camera.pan(from, to, VIEWPORT);
        let now = camera.unproject_ground(to, VIEWPORT).unwrap();
        assert!((now[0] - grabbed[0]).abs() < 0.01 && (now[1] - grabbed[1]).abs() < 0.01);
    }

    #[test]
    fn zooming_keeps_the_ground_point_under_the_cursor_and_clamps_height() {
        let mut camera = REFERENCE;
        let cursor = [1500.0, 300.0];
        let grabbed = camera.unproject_ground(cursor, VIEWPORT).unwrap();
        camera.zoom(2.0, cursor, VIEWPORT);
        let now = camera.unproject_ground(cursor, VIEWPORT).unwrap();
        assert!((now[0] - grabbed[0]).abs() < 0.01 && (now[1] - grabbed[1]).abs() < 0.01);
        assert!((camera.height - 310.178 / 2.0).abs() < 1e-3);
        camera.zoom(0.0001, cursor, VIEWPORT);
        assert_eq!(camera.height, MAX_HEIGHT);
        camera.zoom(1000.0, cursor, VIEWPORT);
        assert_eq!(camera.height, MIN_HEIGHT);
    }

    #[test]
    fn fit_centres_the_map_and_saturates_at_the_engine_ceiling() {
        let camera = Camera::fit([MAP_SIZE_X, MAP_SIZE_Y], VIEWPORT);
        assert_eq!(camera.center, [2816.0, 1024.0]);
        // 3000 is CAMERA_MAX_HEIGHT and shows about 4400 x 2475 units at 16:9.
        assert_eq!(camera.height, MAX_HEIGHT);
        let camera = Camera::fit([400.0, 200.0], VIEWPORT);
        assert!(camera.height > MIN_HEIGHT && camera.height < MAX_HEIGHT);
        let rect = camera.visible_ground(VIEWPORT, 0.0).unwrap();
        assert!(rect[0] <= 0.0 && rect[1] <= 0.0 && rect[2] >= 400.0 && rect[3] >= 200.0);
    }

    #[test]
    fn a_degenerate_camera_is_refused() {
        assert!(
            Camera {
                center: [f32::NAN, 0.0],
                height: 310.0
            }
            .validate()
            .is_err()
        );
        assert!(
            Camera {
                center: [0.0, 0.0],
                height: 9.5
            }
            .validate()
            .is_err()
        );
        assert!(REFERENCE.validate().is_ok());
    }
}
