use crate::*;
use portable_content_manifest::UsedContentKey;
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::fs;
use std::path::Path;

pub const VIEWER_MAP_CAMERA_SCHEMA: &str = "hoi4.viewer-map-camera.v1";
pub const XMAP03_CAMERA_LAW_SCHEMA: &str = "hoi4.xmap03.camera-law.a729.v1";
pub const XMAP03_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const XMAP03_ZOOM_IN_ACTION_ID: u32 = 0x4000_004b;
pub const XMAP03_ZOOM_OUT_ACTION_ID: u32 = 0x4000_004e;
pub const DEFAULT_VIEWER_CAMERA_SPEED_SETTING: f32 = 50.0;
const CAMERA_CURSOR_TILT_MAP_HEIGHT_SCALE: f32 = 0.015;
const CAMERA_WHEEL_CURVE_HALF_PI: f32 = 1.570_796_4;
const CAMERA_FOOTPRINT_LOW_HEIGHT_FRACTION: f32 = 0.04;
const CAMERA_FOOTPRINT_LOW_HEIGHT_TOP_SCALE: f32 = 40.0;
const CAMERA_FOOTPRINT_LOW_HEIGHT_BOTTOM_SCALE: f32 = 0.3;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProductionMapCamera {
    pub view_projection_matrix: [[f32; 4]; 4],
    pub position: [f32; 3],
}

impl ProductionMapCamera {
    pub fn new(view_projection_matrix: [[f32; 4]; 4], position: [f32; 3]) -> Result<Self> {
        let camera = Self {
            view_projection_matrix,
            position,
        };
        camera.validate()?;
        Ok(camera)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self
            .view_projection_matrix
            .iter()
            .flatten()
            .copied()
            .chain(self.position)
            .any(|value| !value.is_finite())
        {
            return Err(
                "production map renderer blocked: app-owned camera contains non-finite values"
                    .into(),
            );
        }
        Ok(())
    }

    pub fn screen_to_ground_y0(
        &self,
        screen_x: f32,
        screen_y: f32,
        screen_w: f32,
        screen_h: f32,
    ) -> Result<Option<(f32, f32)>> {
        self.validate()?;
        if !screen_x.is_finite()
            || !screen_y.is_finite()
            || !screen_w.is_finite()
            || !screen_h.is_finite()
            || screen_w <= 0.0
            || screen_h <= 0.0
        {
            return Err(
                "production map renderer blocked: screen-to-map projection got invalid screen coordinates"
                    .into(),
            );
        }
        let inverse = invert_float4x4(self.view_projection_matrix)?;
        let ndc_x = screen_x / screen_w * 2.0 - 1.0;
        let ndc_y = 1.0 - screen_y / screen_h * 2.0;
        let near = perspective_divide(row_vec4_mul_float4x4([ndc_x, ndc_y, 0.0, 1.0], inverse))?;
        let far = perspective_divide(row_vec4_mul_float4x4([ndc_x, ndc_y, 1.0, 1.0], inverse))?;
        let dir = [far[0] - near[0], far[1] - near[1], far[2] - near[2]];
        if dir[1].abs() <= 1.0e-6 {
            return Ok(None);
        }
        let t = -near[1] / dir[1];
        if !t.is_finite() || t < 0.0 {
            return Ok(None);
        }
        let x = near[0] + dir[0] * t;
        let z = near[2] + dir[2] * t;
        if x.is_finite() && z.is_finite() {
            Ok(Some((x, z)))
        } else {
            Ok(None)
        }
    }

    pub fn world_to_screen(
        &self,
        world: [f32; 3],
        screen_w: f32,
        screen_h: f32,
    ) -> Option<(f32, f32)> {
        self.world_to_screen_depth(world, screen_w, screen_h)
            .map(|screen| (screen[0], screen[1]))
    }

    pub fn world_to_screen_depth(
        &self,
        world: [f32; 3],
        screen_w: f32,
        screen_h: f32,
    ) -> Option<[f32; 3]> {
        if screen_w <= 0.0 || screen_h <= 0.0 {
            return None;
        }
        let clip = row_vec4_mul_float4x4(
            [world[0], world[1], world[2], 1.0],
            self.view_projection_matrix,
        );
        if !(clip[3] > 1.0e-6) {
            return None;
        }
        let ndc_x = clip[0] / clip[3];
        let ndc_y = clip[1] / clip[3];
        let depth = clip[2] / clip[3];
        let sx = (ndc_x * 0.5 + 0.5) * screen_w;
        let sy = (0.5 - ndc_y * 0.5) * screen_h;
        (sx.is_finite() && sy.is_finite() && depth.is_finite()).then_some([sx, sy, depth])
    }
}

const ENGINE_CAMERA_TILT_TABLE: &[(f32, f32)] = &[
    (50.00, 37.0370),
    (50.13, 37.0150),
    (62.02, 35.4263),
    (67.36, 34.9255),
    (79.94, 34.0461),
    (84.29, 33.8108),
    (104.77, 32.9913),
    (120.00, 32.5793),
    (131.37, 32.3385),
    (153.51, 31.9789),
    (168.24, 31.7953),
    (193.46, 31.5493),
    (232.56, 31.2777),
    (252.79, 31.1714),
    (297.87, 30.9884),
    (348.23, 30.8413),
    (399.10, 30.7316),
    (460.09, 30.6324),
    (547.68, 30.5296),
    (646.72, 30.4472),
    (769.19, 30.3750),
    (906.61, 30.3177),
    (1056.66, 30.2722),
    (1239.12, 30.2317),
    (1497.39, 30.1916),
    (1803.07, 30.1589),
    (2086.53, 30.1371),
    (2494.76, 30.1146),
    (3000.00, 30.0954),
];

pub fn engine_camera_lookat_dz(height: f32) -> f32 {
    let table = ENGINE_CAMERA_TILT_TABLE;
    if height <= table[0].0 {
        return table[0].1;
    }
    if height >= table[table.len() - 1].0 {
        return table[table.len() - 1].1;
    }
    for pair in table.windows(2) {
        let (h0, d0) = pair[0];
        let (h1, d1) = pair[1];
        if height <= h1 {
            let t = (height - h0) / (h1 - h0);
            return d0 + (d1 - d0) * t;
        }
    }
    table[table.len() - 1].1
}

pub const ENGINE_MAP_CAMERA_FOV_Y_RAD: f32 = 0.785;
pub const ENGINE_MAP_CAMERA_NEAR: f32 = 5.0;
pub const ENGINE_MAP_CAMERA_FAR: f32 = 4000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProductionMapCameraProjection {
    x_scale: f32,
    y_scale: f32,
    depth_a: f32,
    depth_b: f32,
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
}

impl ProductionMapCameraProjection {
    pub fn from_camera(camera: ProductionMapCamera) -> Result<Self> {
        camera.validate()?;
        let matrix = camera.view_projection_matrix;
        let col0 = matrix_column3(matrix, 0);
        let col1 = matrix_column3(matrix, 1);
        let col2 = matrix_column3(matrix, 2);
        let col3 = matrix_column3(matrix, 3);
        let x_scale = vec3_len(col0);
        let y_scale = vec3_len(col1);
        if !x_scale.is_finite() || !y_scale.is_finite() || x_scale <= 0.0 || y_scale <= 0.0 {
            return Err(
                "production map renderer blocked: captured ViewProjectionMatrix has invalid projection scale"
                    .into(),
            );
        }
        let right = vec3_scale(col0, 1.0 / x_scale);
        let up = vec3_scale(col1, 1.0 / y_scale);
        let forward = vec3_normalize(col3).ok_or(
            "production map renderer blocked: captured ViewProjectionMatrix has no forward axis",
        )?;
        let depth_a = vec3_dot(col2, forward);
        let z_offset = matrix[3][3];
        let depth_b = matrix[3][2] - z_offset * depth_a;
        let projection = Self {
            x_scale,
            y_scale,
            depth_a,
            depth_b,
            right,
            up,
            forward,
        };
        projection.validate()?;
        Ok(projection)
    }

    pub fn from_engine_constants(aspect_ratio: f32) -> Result<Self> {
        if !aspect_ratio.is_finite() || aspect_ratio <= 0.0 {
            return Err("production map renderer blocked: camera aspect ratio is invalid".into());
        }
        let y_scale = 1.0 / (ENGINE_MAP_CAMERA_FOV_Y_RAD * 0.5).tan();
        let x_scale = y_scale / aspect_ratio;
        let near = ENGINE_MAP_CAMERA_NEAR;
        let far = ENGINE_MAP_CAMERA_FAR;
        let span = far - near;
        let depth_a = far / span;
        let depth_b = -near * far / span;
        let far_h = ENGINE_CAMERA_TILT_TABLE[ENGINE_CAMERA_TILT_TABLE.len() - 1].0;
        let dz = engine_camera_lookat_dz(far_h);
        let len = (far_h * far_h + dz * dz).sqrt();
        let forward = [0.0, -far_h / len, dz / len];
        let right = [1.0, 0.0, 0.0];
        let up = [0.0, dz / len, far_h / len];
        let projection = Self {
            x_scale,
            y_scale,
            depth_a,
            depth_b,
            right,
            up,
            forward,
        };
        projection.validate()?;
        Ok(projection)
    }

    pub fn fov_y_degrees(&self) -> f32 {
        self.fov_y_radians().to_degrees()
    }

    pub fn fov_y_radians(&self) -> f32 {
        2.0 * (1.0 / self.y_scale).atan()
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.y_scale / self.x_scale
    }

    pub fn near_z(&self) -> f32 {
        -self.depth_b / self.depth_a
    }

    pub fn far_z(&self) -> f32 {
        self.depth_b / (1.0 - self.depth_a)
    }

    pub fn pitch_down_degrees(&self) -> f32 {
        let horizontal = (self.forward[0] * self.forward[0] + self.forward[2] * self.forward[2])
            .sqrt()
            .max(f32::MIN_POSITIVE);
        (-self.forward[1]).atan2(horizontal).to_degrees()
    }

    fn tilt_basis(&self, height: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
        if !(height > 0.0) {
            return (self.right, self.up, self.forward);
        }
        let dz = engine_camera_lookat_dz(height);
        let len = (height * height + dz * dz).sqrt();
        let forward = [0.0, -height / len, dz / len];
        let right = [1.0, 0.0, 0.0];
        let up = [0.0, dz / len, height / len];
        (right, up, forward)
    }

    pub fn height_for_screen_zoom(
        &self,
        zoom: f32,
        screen_w: f32,
        min_height: f32,
        max_height: f32,
    ) -> Result<f32> {
        self.validate()?;
        if !zoom.is_finite() || zoom <= 0.0 || !screen_w.is_finite() || screen_w <= 0.0 {
            return Err(
                "production map renderer blocked: app camera zoom/screen width is invalid".into(),
            );
        }
        if !min_height.is_finite()
            || !max_height.is_finite()
            || min_height <= 0.0
            || max_height <= min_height
        {
            return Err(
                "production map renderer blocked: the game camera height clamp is invalid".into(),
            );
        }
        let t = (screen_w * 0.5 * self.x_scale) / zoom;
        let mut height = t;
        for _ in 0..3 {
            let dz = engine_camera_lookat_dz(height);
            let squared = t * t - dz * dz;
            height = squared.max(1.0).sqrt();
        }
        Ok(height.clamp(min_height, max_height))
    }

    pub fn screen_zoom_for_height(&self, height: f32, screen_w: f32) -> Result<f32> {
        self.validate()?;
        if !height.is_finite() || height <= 0.0 || !screen_w.is_finite() || screen_w <= 0.0 {
            return Err(
                "production map renderer blocked: camera height/screen width is invalid".into(),
            );
        }
        let dz = engine_camera_lookat_dz(height);
        let t = (height * height + dz * dz).sqrt();
        Ok((screen_w * 0.5 * self.x_scale) / t)
    }

    pub fn camera_for_ground_center(
        &self,
        center_x: f32,
        center_z: f32,
        height: f32,
    ) -> Result<ProductionMapCamera> {
        self.validate()?;
        if !center_x.is_finite() || !center_z.is_finite() || !height.is_finite() || height <= 0.0 {
            return Err(
                "production map renderer blocked: app camera center/height is invalid".into(),
            );
        }
        let dz = engine_camera_lookat_dz(height);
        let position = [center_x, height, center_z - dz];
        self.camera_from_position(position)
    }

    pub fn camera_from_position(&self, position: [f32; 3]) -> Result<ProductionMapCamera> {
        self.validate()?;
        if position.iter().any(|value| !value.is_finite()) {
            return Err(
                "production map renderer blocked: app-owned camera position is non-finite".into(),
            );
        }
        let (right, up, forward) = self.tilt_basis(position[1]);
        let dot_right = vec3_dot(right, position);
        let dot_up = vec3_dot(up, position);
        let dot_forward = vec3_dot(forward, position);
        ProductionMapCamera::new(
            [
                [
                    right[0] * self.x_scale,
                    up[0] * self.y_scale,
                    forward[0] * self.depth_a,
                    forward[0],
                ],
                [
                    right[1] * self.x_scale,
                    up[1] * self.y_scale,
                    forward[1] * self.depth_a,
                    forward[1],
                ],
                [
                    right[2] * self.x_scale,
                    up[2] * self.y_scale,
                    forward[2] * self.depth_a,
                    forward[2],
                ],
                [
                    -dot_right * self.x_scale,
                    -dot_up * self.y_scale,
                    -dot_forward * self.depth_a + self.depth_b,
                    -dot_forward,
                ],
            ],
            position,
        )
    }

    pub fn camera_from_eye_look_at(
        &self,
        position: [f32; 3],
        look_at: [f32; 3],
    ) -> Result<ProductionMapCamera> {
        self.validate()?;
        if position
            .into_iter()
            .chain(look_at)
            .any(|value| !value.is_finite())
        {
            return Err("production map renderer blocked: camera eye/look-at is non-finite".into());
        }
        let forward = vec3_normalize([
            look_at[0] - position[0],
            look_at[1] - position[1],
            look_at[2] - position[2],
        ])
        .ok_or("production map renderer blocked: camera eye/look-at is degenerate")?;
        let right = vec3_normalize([forward[2], 0.0, -forward[0]])
            .ok_or("production map renderer blocked: camera forward is vertical")?;
        let up = [
            forward[1] * right[2] - forward[2] * right[1],
            forward[2] * right[0] - forward[0] * right[2],
            forward[0] * right[1] - forward[1] * right[0],
        ];
        let dot_right = vec3_dot(right, position);
        let dot_up = vec3_dot(up, position);
        let dot_forward = vec3_dot(forward, position);
        ProductionMapCamera::new(
            [
                [
                    right[0] * self.x_scale,
                    up[0] * self.y_scale,
                    forward[0] * self.depth_a,
                    forward[0],
                ],
                [
                    right[1] * self.x_scale,
                    up[1] * self.y_scale,
                    forward[1] * self.depth_a,
                    forward[1],
                ],
                [
                    right[2] * self.x_scale,
                    up[2] * self.y_scale,
                    forward[2] * self.depth_a,
                    forward[2],
                ],
                [
                    -dot_right * self.x_scale,
                    -dot_up * self.y_scale,
                    -dot_forward * self.depth_a + self.depth_b,
                    -dot_forward,
                ],
            ],
            position,
        )
    }

    fn validate(&self) -> Result<()> {
        if [
            self.x_scale,
            self.y_scale,
            self.depth_a,
            self.depth_b,
            self.near_z(),
            self.far_z(),
        ]
        .into_iter()
        .chain(self.right)
        .chain(self.up)
        .chain(self.forward)
        .any(|value| !value.is_finite())
        {
            return Err(
                "production map renderer blocked: decomposed camera projection is non-finite"
                    .into(),
            );
        }
        if self.x_scale <= 0.0
            || self.y_scale <= 0.0
            || self.near_z() <= 0.0
            || self.far_z() <= self.near_z()
            || -self.forward[1] <= 0.0
        {
            return Err(
                "production map renderer blocked: decomposed camera projection is invalid".into(),
            );
        }
        Ok(())
    }
}

pub fn map_wrap_offsets(view_min_x: f32, view_max_x: f32, map_width: f32) -> Vec<f32> {
    if !view_min_x.is_finite()
        || !view_max_x.is_finite()
        || !map_width.is_finite()
        || map_width <= 0.0
        || view_min_x > view_max_x
    {
        return Vec::new();
    }

    let view_width = view_max_x - view_min_x;
    if !view_width.is_finite() {
        return Vec::new();
    }
    if view_width >= map_width {
        return vec![0.0, -map_width, map_width];
    }

    [0.0, -map_width, map_width]
        .into_iter()
        .filter(|offset| ranges_intersect(*offset, *offset + map_width, view_min_x, view_max_x))
        .collect()
}

fn ranges_intersect(a_min: f32, a_max: f32, b_min: f32, b_max: f32) -> bool {
    a_max >= b_min && a_min <= b_max
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hoi4MapCameraDefines {
    pub(crate) min_height: f32,
    pub(crate) max_height: f32,
    pub(crate) outside_map_distance_top: f32,
    pub(crate) outside_map_distance_bottom: f32,
    pub(crate) zoom_speed: f32,
    pub(crate) zoom_key_scale: f32,
    pub(crate) zoom_speed_distance_mult: f32,
}

impl Hoi4MapCameraDefines {
    pub(crate) fn new(
        min_height: f32,
        max_height: f32,
        outside_map_distance_top: f32,
        outside_map_distance_bottom: f32,
        zoom_speed: f32,
        zoom_key_scale: f32,
        zoom_speed_distance_mult: f32,
    ) -> Result<Self> {
        let defines = Self {
            min_height,
            max_height,
            outside_map_distance_top,
            outside_map_distance_bottom,
            zoom_speed,
            zoom_key_scale,
            zoom_speed_distance_mult,
        };
        defines.validate()?;
        Ok(defines)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if !self.min_height.is_finite()
            || !self.max_height.is_finite()
            || self.min_height <= 0.0
            || self.max_height <= self.min_height
        {
            return Err(format!(
                "production map viewer blocked: {BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} has invalid camera height range {}..{}",
                self.min_height, self.max_height
            )
            .into());
        }
        for (name, value) in [
            (
                "CAMERA_OUTSIDE_MAP_DISTANCE_TOP",
                self.outside_map_distance_top,
            ),
            (
                "CAMERA_OUTSIDE_MAP_DISTANCE_BOTTOM",
                self.outside_map_distance_bottom,
            ),
            ("CAMERA_ZOOM_SPEED", self.zoom_speed),
            ("CAMERA_ZOOM_KEY_SCALE", self.zoom_key_scale),
            (
                "CAMERA_ZOOM_SPEED_DISTANCE_MULT",
                self.zoom_speed_distance_mult,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!(
                    "production map viewer blocked: {BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} {name} has invalid value {value}"
                )
                .into());
            }
        }
        Ok(())
    }

    pub fn min_height(&self) -> f32 {
        self.min_height
    }

    pub fn max_height(&self) -> f32 {
        self.max_height
    }

    pub fn outside_map_distance_top(&self) -> f32 {
        self.outside_map_distance_top
    }

    pub fn outside_map_distance_bottom(&self) -> f32 {
        self.outside_map_distance_bottom
    }

    pub fn zoom_speed(&self) -> f32 {
        self.zoom_speed
    }

    pub fn zoom_key_scale(&self) -> f32 {
        self.zoom_key_scale
    }

    pub fn zoom_speed_distance_mult(&self) -> f32 {
        self.zoom_speed_distance_mult
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewerMapCameraState {
    pub schema: String,
    pub engine_law_schema: String,
    pub engine_executable_sha256: String,
    pub graphics_defines_content: UsedContentKey,
    pub map_width: u32,
    pub map_height: u32,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub camera_speed_setting_f32_bits: u32,
    pub camera_min_height_f32_bits: u32,
    pub camera_max_height_f32_bits: u32,
    pub outside_map_distance_top_f32_bits: u32,
    pub outside_map_distance_bottom_f32_bits: u32,
    pub zoom_speed_f32_bits: u32,
    pub zoom_key_scale_f32_bits: u32,
    pub zoom_speed_distance_mult_f32_bits: u32,
    pub current_eye_f32_bits: [u32; 3],
    pub current_look_at_f32_bits: [u32; 3],
    pub target_eye_f32_bits: [u32; 3],
    pub target_look_at_f32_bits: [u32; 3],
    pub input_revision: u64,
    pub deterministic_sha256: String,
}

impl ViewerMapCameraState {
    pub fn from_admitted_content(
        resolver: &GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        map_width: u32,
        map_height: u32,
        viewport_width: u32,
        viewport_height: u32,
        camera_speed_setting: f32,
    ) -> Result<Self> {
        let content = admission
            .require_used_content(BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
            .clone();
        let bytes = resolver.read_admitted_used_content(&content)?;
        let defines = hoi4_map_camera_defines_from_source(&latin1_bytes_to_string(&bytes))?;
        Self::new(
            content,
            defines,
            map_width,
            map_height,
            viewport_width,
            viewport_height,
            camera_speed_setting,
        )
    }

    pub fn new(
        graphics_defines_content: UsedContentKey,
        defines: Hoi4MapCameraDefines,
        map_width: u32,
        map_height: u32,
        viewport_width: u32,
        viewport_height: u32,
        camera_speed_setting: f32,
    ) -> Result<Self> {
        defines.validate()?;
        if map_width == 0 || map_height == 0 || viewport_width == 0 || viewport_height == 0 {
            return Err("viewer map camera has a zero map or viewport extent".into());
        }
        if !camera_speed_setting.is_finite() || !(0.0..=100.0).contains(&camera_speed_setting) {
            return Err("viewer map camera speed setting is outside 0..=100".into());
        }
        let center = [map_width as f32 * 0.5, 0.0, map_height as f32 * 0.5];
        let height = defines.max_height();
        let eye = [
            center[0],
            height,
            center[2] - engine_camera_lookat_dz(height),
        ];
        let mut state = Self {
            schema: VIEWER_MAP_CAMERA_SCHEMA.into(),
            engine_law_schema: XMAP03_CAMERA_LAW_SCHEMA.into(),
            engine_executable_sha256: XMAP03_A729_EXECUTABLE_SHA256.into(),
            graphics_defines_content,
            map_width,
            map_height,
            viewport_width,
            viewport_height,
            camera_speed_setting_f32_bits: camera_speed_setting.to_bits(),
            camera_min_height_f32_bits: defines.min_height().to_bits(),
            camera_max_height_f32_bits: defines.max_height().to_bits(),
            outside_map_distance_top_f32_bits: defines.outside_map_distance_top().to_bits(),
            outside_map_distance_bottom_f32_bits: defines.outside_map_distance_bottom().to_bits(),
            zoom_speed_f32_bits: defines.zoom_speed().to_bits(),
            zoom_key_scale_f32_bits: defines.zoom_key_scale().to_bits(),
            zoom_speed_distance_mult_f32_bits: defines.zoom_speed_distance_mult().to_bits(),
            current_eye_f32_bits: eye.map(f32::to_bits),
            current_look_at_f32_bits: center.map(f32::to_bits),
            target_eye_f32_bits: eye.map(f32::to_bits),
            target_look_at_f32_bits: center.map(f32::to_bits),
            input_revision: 0,
            deterministic_sha256: String::new(),
        };
        state.repair_projected_footprint()?;
        state.seal()
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        if self.deterministic_sha256.len() != 64
            || !self
                .deterministic_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("viewer map camera digest is not canonical SHA-256".into());
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("viewer map camera payload digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        if self.schema != VIEWER_MAP_CAMERA_SCHEMA
            || self.engine_law_schema != XMAP03_CAMERA_LAW_SCHEMA
            || self.engine_executable_sha256 != XMAP03_A729_EXECUTABLE_SHA256
        {
            return Err("viewer map camera engine-law identity mismatch".into());
        }
        self.graphics_defines_content
            .validate()
            .map_err(|error| format!("viewer camera graphics defines identity: {error}"))?;
        if self.graphics_defines_content.normalized_game_relative_path
            != BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE
            || self.map_width == 0
            || self.map_height == 0
            || self.viewport_width == 0
            || self.viewport_height == 0
        {
            return Err("viewer map camera content/path/extents are invalid".into());
        }
        self.defines()?.validate()?;
        let speed = f32::from_bits(self.camera_speed_setting_f32_bits);
        if !speed.is_finite() || !(0.0..=100.0).contains(&speed) {
            return Err("viewer map camera speed setting is outside 0..=100".into());
        }
        for value in self
            .current_eye()
            .into_iter()
            .chain(self.current_look_at())
            .chain(self.target_eye())
            .chain(self.target_look_at())
        {
            if !value.is_finite() {
                return Err("viewer map camera contains non-finite state".into());
            }
        }
        let min = self.defines()?.min_height();
        let max = self.defines()?.max_height();
        if !(min..=max).contains(&self.current_eye()[1])
            || !(min..=max).contains(&self.target_eye()[1])
        {
            return Err("viewer map camera height escaped admitted clamps".into());
        }
        self.production_camera()?.validate()
    }

    fn seal(mut self) -> Result<Self> {
        self.deterministic_sha256.clear();
        self.validate_body()?;
        self.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&self)?);
        self.validate()?;
        Ok(self)
    }

    fn reseal(&mut self) -> Result<()> {
        self.deterministic_sha256.clear();
        self.validate_body()?;
        self.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(self)?);
        self.validate()
    }

    fn defines(&self) -> Result<Hoi4MapCameraDefines> {
        Hoi4MapCameraDefines::new(
            f32::from_bits(self.camera_min_height_f32_bits),
            f32::from_bits(self.camera_max_height_f32_bits),
            f32::from_bits(self.outside_map_distance_top_f32_bits),
            f32::from_bits(self.outside_map_distance_bottom_f32_bits),
            f32::from_bits(self.zoom_speed_f32_bits),
            f32::from_bits(self.zoom_key_scale_f32_bits),
            f32::from_bits(self.zoom_speed_distance_mult_f32_bits),
        )
    }

    fn projection(&self) -> Result<ProductionMapCameraProjection> {
        ProductionMapCameraProjection::from_engine_constants(
            self.viewport_width as f32 / self.viewport_height as f32,
        )
    }

    pub fn current_eye(&self) -> [f32; 3] {
        self.current_eye_f32_bits.map(f32::from_bits)
    }

    pub fn current_look_at(&self) -> [f32; 3] {
        self.current_look_at_f32_bits.map(f32::from_bits)
    }

    pub fn target_eye(&self) -> [f32; 3] {
        self.target_eye_f32_bits.map(f32::from_bits)
    }

    pub fn target_look_at(&self) -> [f32; 3] {
        self.target_look_at_f32_bits.map(f32::from_bits)
    }

    pub fn production_camera(&self) -> Result<ProductionMapCamera> {
        self.projection()?
            .camera_from_eye_look_at(self.current_eye(), self.current_look_at())
    }

    pub fn inverse_view_projection_matrix(&self) -> Result<[[f32; 4]; 4]> {
        invert_float4x4(self.production_camera()?.view_projection_matrix)
    }

    pub fn current_look_at_direction(&self) -> Result<[f32; 3]> {
        let eye = self.current_eye();
        let look = self.current_look_at();
        vec3_normalize([look[0] - eye[0], look[1] - eye[1], look[2] - eye[2]])
            .ok_or_else(|| "viewer map camera has a degenerate look-at direction".into())
    }

    pub fn center_map_pixels(&self) -> (f64, f64) {
        let look = self.current_look_at();
        (
            f64::from(look[0].rem_euclid(self.map_width as f32)),
            f64::from(self.map_height as f32 - look[2]),
        )
    }

    pub fn screen_zoom(&self) -> Result<f64> {
        Ok(f64::from(self.projection()?.screen_zoom_for_height(
            self.current_eye()[1],
            self.viewport_width as f32,
        )?))
    }

    pub fn screen_to_map(&self, screen_x: f32, screen_y: f32) -> Result<Option<(f32, f32)>> {
        Ok(self
            .production_camera()?
            .screen_to_ground_y0(
                screen_x,
                screen_y,
                self.viewport_width as f32,
                self.viewport_height as f32,
            )?
            .map(|(x, z)| {
                (
                    x.rem_euclid(self.map_width as f32),
                    self.map_height as f32 - z,
                )
            }))
    }

    pub fn resize_viewport(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Err("viewer map camera resize has zero extent".into());
        }
        self.viewport_width = width;
        self.viewport_height = height;
        self.input_revision = self.input_revision.saturating_add(1);
        self.repair_projected_footprint()?;
        self.reseal()
    }

    pub fn reset_to_engine_overview(&mut self) -> Result<()> {
        let defines = self.defines()?;
        let look = [
            self.map_width as f32 * 0.5,
            0.0,
            self.map_height as f32 * 0.5,
        ];
        let height = defines.max_height();
        let eye = [look[0], height, look[2] - engine_camera_lookat_dz(height)];
        self.current_eye_f32_bits = eye.map(f32::to_bits);
        self.current_look_at_f32_bits = look.map(f32::to_bits);
        self.target_eye_f32_bits = eye.map(f32::to_bits);
        self.target_look_at_f32_bits = look.map(f32::to_bits);
        self.input_revision = self.input_revision.saturating_add(1);
        self.repair_projected_footprint()?;
        self.reseal()
    }

    pub fn pan_screen(&mut self, delta_x: f32, delta_y: f32) -> Result<()> {
        if !delta_x.is_finite() || !delta_y.is_finite() {
            return Err("viewer camera pan delta is non-finite".into());
        }
        let camera = self.production_camera()?;
        let cx = self.viewport_width as f32 * 0.5;
        let cy = self.viewport_height as f32 * 0.5;
        let center = camera
            .screen_to_ground_y0(
                cx,
                cy,
                self.viewport_width as f32,
                self.viewport_height as f32,
            )?
            .ok_or("viewer camera center ray misses the map plane")?;
        let moved = camera
            .screen_to_ground_y0(
                cx + delta_x,
                cy + delta_y,
                self.viewport_width as f32,
                self.viewport_height as f32,
            )?
            .ok_or("viewer camera pan ray misses the map plane")?;
        let shift = [moved.0 - center.0, 0.0, moved.1 - center.1];
        let mut eye = self.target_eye();
        let mut look = self.target_look_at();
        for component in 0..3 {
            eye[component] += shift[component];
            look[component] += shift[component];
        }
        normalize_wrapped_pair(&mut eye, &mut look, self.map_width as f32);
        self.target_eye_f32_bits = eye.map(f32::to_bits);
        self.target_look_at_f32_bits = look.map(f32::to_bits);
        self.input_revision = self.input_revision.saturating_add(1);
        self.reseal()
    }

    pub fn key_zoom_action(&mut self, runtime_action_id: u32) -> Result<bool> {
        let sign = match runtime_action_id {
            XMAP03_ZOOM_IN_ACTION_ID => -1.0,
            XMAP03_ZOOM_OUT_ACTION_ID => 1.0,
            _ => return Err("viewer camera received an unknown C03 zoom action id".into()),
        };
        let defines = self.defines()?;
        let step0 = defines.zoom_speed() * f32::from_bits(self.camera_speed_setting_f32_bits);
        let signed_step = step0 * defines.zoom_key_scale() * sign;
        let proposed_height = self.current_eye()[1] + signed_step;
        let old_target_eye = self.target_eye();
        let mut next_target_eye = old_target_eye;
        next_target_eye[1] = (proposed_height - old_target_eye[1]) + old_target_eye[1];
        next_target_eye[1] = next_target_eye[1].clamp(defines.min_height(), defines.max_height());
        if (old_target_eye[1] - next_target_eye[1]).abs() < 1.0 {
            return Ok(false);
        }
        let mut next_target_look = [
            next_target_eye[0],
            0.0,
            next_target_eye[2] + engine_camera_lookat_dz(next_target_eye[1]),
        ];
        normalize_wrapped_pair(
            &mut next_target_eye,
            &mut next_target_look,
            self.map_width as f32,
        );
        self.target_eye_f32_bits = next_target_eye.map(f32::to_bits);
        self.target_look_at_f32_bits = next_target_look.map(f32::to_bits);
        self.input_revision = self.input_revision.saturating_add(1);
        self.reseal()?;
        Ok(true)
    }

    pub fn wheel_zoom(
        &mut self,
        screen_x: f32,
        screen_y: f32,
        signed_delta: i32,
        raw_owner_delta: f32,
    ) -> Result<()> {
        if !raw_owner_delta.is_finite() || raw_owner_delta < 0.0 {
            return Err("viewer camera wheel delta is invalid".into());
        }
        if signed_delta == 0 {
            return Ok(());
        }
        let defines = self.defines()?;
        let current_eye = self.current_eye();
        let min_height = defines.min_height();
        let max_height = defines.max_height();
        let height_span = max_height - min_height;
        let h = ((current_eye[1] - min_height) / height_span).clamp(0.0, 1.0);
        if (h >= 1.0 && signed_delta < 0) || (h <= 0.0 && signed_delta > 0) {
            return Ok(());
        }
        let ground = self
            .production_camera()?
            .screen_to_ground_y0(
                screen_x,
                screen_y,
                self.viewport_width as f32,
                self.viewport_height as f32,
            )?
            .ok_or("viewer camera wheel ray misses the map plane")?;
        let cursor_target = [
            ground.0,
            0.0,
            ground.1 - self.map_height as f32 * CAMERA_CURSOR_TILT_MAP_HEIGHT_SCALE * (1.0 - h),
        ];
        let ray = vec3_normalize([
            cursor_target[0] - current_eye[0],
            cursor_target[1] - current_eye[1],
            cursor_target[2] - current_eye[2],
        ])
        .ok_or("viewer camera wheel cursor ray is degenerate")?;
        let frame = if raw_owner_delta <= CAMERA_RAW_DELTA_CAP {
            raw_owner_delta
        } else {
            CAMERA_RAW_DELTA_CAP
        };
        let base0 = frame * f32::from_bits(self.camera_speed_setting_f32_bits);
        let base = base0 * defines.zoom_speed();
        let curve_input = h * CAMERA_WHEEL_CURVE_HALF_PI - CAMERA_WHEEL_CURVE_HALF_PI;
        let sine = curve_input.sin();
        let speed = ((sine + 1.0) * defines.zoom_speed_distance_mult()) * base + base;
        let raw = signed_delta as f32;
        let mut delta = [
            ray[0] * speed * raw,
            ray[1] * speed * raw,
            ray[2] * speed * raw,
        ];
        let mut target_eye = self.target_eye();
        let proposed_height = target_eye[1] + delta[1];
        if proposed_height <= min_height {
            delta[1] = min_height - target_eye[1];
            let horizontal_scale = h + 0.1;
            delta[0] *= horizontal_scale;
            delta[2] *= horizontal_scale;
        }
        for component in 0..3 {
            target_eye[component] += delta[component];
        }
        target_eye[1] = target_eye[1].clamp(min_height, max_height);
        let target_look = [
            target_eye[0],
            0.0,
            target_eye[2] + engine_camera_lookat_dz(target_eye[1]),
        ];
        let mut target_look = target_look;
        normalize_wrapped_pair(&mut target_eye, &mut target_look, self.map_width as f32);
        self.target_eye_f32_bits = target_eye.map(f32::to_bits);
        self.target_look_at_f32_bits = target_look.map(f32::to_bits);
        self.input_revision = self.input_revision.saturating_add(1);
        self.reseal()
    }

    pub fn advance(&mut self, raw_owner_delta: f32) -> Result<()> {
        if !raw_owner_delta.is_finite() || raw_owner_delta < 0.0 {
            return Err("viewer camera temporal delta is invalid".into());
        }
        let defines = self.defines()?;
        let outcome = camera_temporal_update_and_bounds_repair(
            CameraTemporalState {
                current_eye: self.current_eye(),
                current_look_at: self.current_look_at(),
                target_eye: self.target_eye(),
                target_look_at: self.target_look_at(),
            },
            CameraTemporalInputs {
                raw_owner_delta,
                ramp_6f4: 1.0,
                settings_camera_speed: f32::from_bits(self.camera_speed_setting_f32_bits),
                interpolation_speed: CAMERA_INTERPOLATION_SPEED,
                camera_min_height_i32: defines.min_height() as i32,
                camera_max_height_i32: defines.max_height() as i32,
                active_map_min_height_i32: 0,
                map_x_max_i32: i32::try_from(self.map_width)
                    .map_err(|_| "viewer camera map width exceeds exact i32 law")?,
                map_z_max_i32: i32::try_from(self.map_height)
                    .map_err(|_| "viewer camera map height exceeds exact i32 law")?,
                wrap_x: true,
            },
        );
        let mut current_eye = outcome.state.current_eye;
        let mut current_look = outcome.state.current_look_at;
        let mut target_eye = outcome.state.target_eye;
        let mut target_look = outcome.state.target_look_at;
        normalize_wrapped_pair(&mut current_eye, &mut current_look, self.map_width as f32);
        normalize_wrapped_pair(&mut target_eye, &mut target_look, self.map_width as f32);
        self.current_eye_f32_bits = current_eye.map(f32::to_bits);
        self.current_look_at_f32_bits = current_look.map(f32::to_bits);
        self.target_eye_f32_bits = target_eye.map(f32::to_bits);
        self.target_look_at_f32_bits = target_look.map(f32::to_bits);
        self.repair_projected_footprint()?;
        self.reseal()
    }

    fn repair_projected_footprint(&mut self) -> Result<()> {
        let defines = self.defines()?;
        let current_eye = self.current_eye();
        let camera = self.production_camera()?;
        let cx = self.viewport_width as f32 * 0.5;
        let top = camera
            .screen_to_ground_y0(
                cx,
                0.0,
                self.viewport_width as f32,
                self.viewport_height as f32,
            )?
            .ok_or("viewer camera top footprint ray misses the map plane")?
            .1;
        let bottom = camera
            .screen_to_ground_y0(
                cx,
                self.viewport_height as f32,
                self.viewport_width as f32,
                self.viewport_height as f32,
            )?
            .ok_or("viewer camera bottom footprint ray misses the map plane")?
            .1;
        let (adjusted_top, adjusted_bottom) = low_height_footprint_intersections(
            top,
            bottom,
            current_eye[1],
            defines.min_height(),
            defines.max_height(),
        );
        let top_limit = self.map_height as f32 + defines.outside_map_distance_top();
        let bottom_with_margin = adjusted_bottom + defines.outside_map_distance_bottom();
        let top_overflow = adjusted_top - top_limit;
        let span = (adjusted_bottom - adjusted_top).abs();
        let allowed_span = top_limit + defines.outside_map_distance_bottom();
        let correction = if span <= allowed_span {
            if top_overflow > 0.0 {
                -top_overflow
            } else if bottom_with_margin < 0.0 {
                -bottom_with_margin
            } else {
                0.0
            }
        } else {
            -((bottom_with_margin + top_overflow) * 0.5)
        };
        if correction != 0.0 {
            let mut eye = current_eye;
            let mut look = self.current_look_at();
            eye[2] += correction;
            look[2] += correction;
            self.current_eye_f32_bits = eye.map(f32::to_bits);
            self.current_look_at_f32_bits = look.map(f32::to_bits);
            let mut target_eye = self.target_eye();
            target_eye[2] = eye[2];
            self.target_eye_f32_bits = target_eye.map(f32::to_bits);
            self.target_look_at_f32_bits = look.map(f32::to_bits);
        }
        Ok(())
    }
}

fn normalize_wrapped_pair(eye: &mut [f32; 3], look: &mut [f32; 3], map_width: f32) {
    let wrapped = look[0].rem_euclid(map_width);
    let correction = wrapped - look[0];
    look[0] = wrapped;
    eye[0] += correction;
}

fn low_height_footprint_intersections(
    mut top: f32,
    mut bottom: f32,
    current_height: f32,
    minimum_height: f32,
    maximum_height: f32,
) -> (f32, f32) {
    let low_height_span = (maximum_height - minimum_height) * CAMERA_FOOTPRINT_LOW_HEIGHT_FRACTION;
    let height_above_minimum = current_height - minimum_height;
    if height_above_minimum < low_height_span {
        let curve_input =
            (1.0 - height_above_minimum / low_height_span) * CAMERA_WHEEL_CURVE_HALF_PI;
        let correction = (1.0 - curve_input.cos()) * CAMERA_FOOTPRINT_LOW_HEIGHT_TOP_SCALE;
        top -= correction;
        bottom -= correction * CAMERA_FOOTPRINT_LOW_HEIGHT_BOTTOM_SCALE;
    }
    (top, bottom)
}

pub fn hoi4_map_camera_defines_from_bundle(
    bundle: &NativeD3d11TerrainBundle,
) -> Result<Hoi4MapCameraDefines> {
    let evidence = bundle
        .frame
        .vanilla_sources
        .get(BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE)
        .ok_or_else(|| {
            format!(
                "production map viewer blocked: map-frame bundle is missing vanilla graphics defines evidence for {BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE}"
            )
        })?;
    let bytes = read_verified_map_frame_file(
        &bundle.bundle_dir,
        &evidence.file,
        evidence.byte_len,
        &evidence.fnv1a64,
        "graphics defines",
    )?;
    hoi4_map_camera_defines_from_source(&latin1_bytes_to_string(&bytes))
}

pub(crate) fn hoi4_map_camera_defines_from_source(source: &str) -> Result<Hoi4MapCameraDefines> {
    Hoi4MapCameraDefines::new(
        parse_lua_number_define(source, "CAMERA_MIN_HEIGHT")?,
        parse_lua_number_define(source, "CAMERA_MAX_HEIGHT")?,
        parse_lua_number_define(source, "CAMERA_OUTSIDE_MAP_DISTANCE_TOP")?,
        parse_lua_number_define(source, "CAMERA_OUTSIDE_MAP_DISTANCE_BOTTOM")?,
        parse_lua_number_define(source, "CAMERA_ZOOM_SPEED")?,
        parse_lua_number_define(source, "CAMERA_ZOOM_KEY_SCALE")?,
        parse_lua_number_define(source, "CAMERA_ZOOM_SPEED_DISTANCE_MULT")?,
    )
}

pub(crate) fn parse_lua_number_define(source: &str, key: &str) -> Result<f32> {
    let mut parsed = None;
    for (line_index, line) in source.lines().enumerate() {
        let code = line.split("--").next().unwrap_or(line).trim();
        let Some(rest) = code.strip_prefix(key) else {
            continue;
        };
        let rest = rest.trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..]
            .trim_start()
            .trim_end_matches(',')
            .split_whitespace()
            .next()
            .ok_or_else(|| {
                format!(
                    "production map viewer blocked: {BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} {key} has no numeric value"
                )
            })?
            .trim_end_matches(',')
            .parse::<f32>()
            .map_err(|err| {
                format!(
                    "production map viewer blocked: {BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} {key} on line {} is not a number: {err}",
                    line_index + 1
                )
            })?;
        if parsed.replace(value).is_some() {
            return Err(format!(
                "production map viewer blocked: {BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} defines {key} more than once"
            )
            .into());
        }
    }
    parsed.ok_or_else(|| {
        format!("production map viewer blocked: {BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} is missing {key}")
            .into()
    })
}

pub(crate) fn production_map_camera_from_json(path: &Path) -> Result<ProductionMapCamera> {
    let camera: ProductionMapCamera = serde_json::from_slice(&fs::read(path)?)?;
    camera.validate()?;
    Ok(camera)
}

pub(crate) fn invert_float4x4(matrix: [[f32; 4]; 4]) -> Result<[[f32; 4]; 4]> {
    let mut augmented = [[0.0f32; 8]; 4];
    for row in 0..4 {
        for column in 0..4 {
            augmented[row][column] = matrix[row][column];
        }
        augmented[row][4 + row] = 1.0;
    }

    for column in 0..4 {
        let mut pivot_row = column;
        let mut pivot_abs = augmented[column][column].abs();
        for candidate in (column + 1)..4 {
            let candidate_abs = augmented[candidate][column].abs();
            if candidate_abs > pivot_abs {
                pivot_abs = candidate_abs;
                pivot_row = candidate;
            }
        }
        if pivot_abs <= 1.0e-8 {
            return Err(
                "production map renderer blocked: app-owned camera ViewProjectionMatrix is singular"
                    .into(),
            );
        }
        if pivot_row != column {
            augmented.swap(column, pivot_row);
        }

        let pivot = augmented[column][column];
        for value in &mut augmented[column] {
            *value /= pivot;
        }

        for row in 0..4 {
            if row == column {
                continue;
            }
            let factor = augmented[row][column];
            if factor == 0.0 {
                continue;
            }
            for column_index in 0..8 {
                augmented[row][column_index] -= factor * augmented[column][column_index];
            }
        }
    }

    let mut inverse = [[0.0f32; 4]; 4];
    for row in 0..4 {
        for column in 0..4 {
            inverse[row][column] = augmented[row][4 + column];
        }
    }
    if inverse.iter().flatten().any(|value| !value.is_finite()) {
        return Err(
            "production map renderer blocked: app-owned camera inverse matrix is non-finite".into(),
        );
    }
    Ok(inverse)
}

fn matrix_column3(matrix: [[f32; 4]; 4], column: usize) -> [f32; 3] {
    [matrix[0][column], matrix[1][column], matrix[2][column]]
}

fn vec3_len(value: [f32; 3]) -> f32 {
    vec3_dot(value, value).sqrt()
}

fn vec3_scale(value: [f32; 3], scale: f32) -> [f32; 3] {
    [value[0] * scale, value[1] * scale, value[2] * scale]
}

fn vec3_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn vec3_normalize(value: [f32; 3]) -> Option<[f32; 3]> {
    let len = vec3_len(value);
    (len.is_finite() && len > 0.0).then(|| vec3_scale(value, 1.0 / len))
}

fn row_vec4_mul_float4x4(value: [f32; 4], matrix: [[f32; 4]; 4]) -> [f32; 4] {
    [
        value[0] * matrix[0][0]
            + value[1] * matrix[1][0]
            + value[2] * matrix[2][0]
            + value[3] * matrix[3][0],
        value[0] * matrix[0][1]
            + value[1] * matrix[1][1]
            + value[2] * matrix[2][1]
            + value[3] * matrix[3][1],
        value[0] * matrix[0][2]
            + value[1] * matrix[1][2]
            + value[2] * matrix[2][2]
            + value[3] * matrix[3][2],
        value[0] * matrix[0][3]
            + value[1] * matrix[1][3]
            + value[2] * matrix[2][3]
            + value[3] * matrix[3][3],
    ]
}

fn perspective_divide(value: [f32; 4]) -> Result<[f32; 3]> {
    if !value.iter().all(|component| component.is_finite()) || value[3].abs() <= 1.0e-6 {
        return Err(
            "production map renderer blocked: screen-to-map unprojection produced invalid homogeneous coordinates"
                .into(),
        );
    }
    Ok([
        value[0] / value[3],
        value[1] / value[3],
        value[2] / value[3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    use std::fs;

    fn assert_close_epsilon(actual: f32, expected: f32, epsilon: f32) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "expected {actual} to be within {epsilon} of {expected}"
        );
    }

    #[test]
    fn invert_float4x4_inverts_app_camera_matrix() {
        let matrix = [
            [2.0, 0.0, 0.0, 4.0],
            [0.0, 4.0, 0.0, 8.0],
            [0.0, 0.0, 5.0, 10.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let inverse = invert_float4x4(matrix).expect("invert affine matrix");

        assert_eq!(
            inverse,
            [
                [0.5, 0.0, 0.0, -2.0],
                [0.0, 0.25, 0.0, -2.0],
                [0.0, 0.0, 0.2, -2.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        );

        let singular = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let err = invert_float4x4(singular).expect_err("singular camera must fail closed");
        assert!(err.to_string().contains("singular"));
    }

    #[test]
    fn engine_sweep_matrices_reconstruct_from_tilt_law() {
        const H2999_M: [[f32; 4]; 4] = [
            [1.358760e0, 0.0, 0.0, 0.0],
            [0.0, 2.423869e-2, -1.001201e0, -9.999496e-1],
            [0.0, 2.415452e0, 1.004690e-2, 1.003434e-2],
            [-3.176648e3, -2.460975e3, 2.987751e3, 2.989016e3],
        ];
        const H2999_POS: [f32; 3] = [2337.9019, 2999.0889, 988.7512];
        const H500_M: [[f32; 4]; 4] = [
            [1.358760e0, 0.0, 0.0, 0.0],
            [0.0, 1.476872e-1, -9.993784e-1, -9.981292e-1],
            [0.0, 2.411055e0, 6.121611e-2, 6.113959e-2],
            [-4.040693e3, -3.558928e3, 4.054576e2, 4.099508e2],
        ];
        const H500_POS: [f32; 3] = [2973.8086, 499.2625, 1445.5060];
        const H50_M: [[f32; 4]; 4] = [
            [1.358760e0, 0.0, 0.0, 0.0],
            [0.0, 1.437806e0, -8.045660e-1, -8.035603e-1],
            [0.0, 1.941059e0, 5.959682e-1, 5.952233e-1],
            [-4.094259e3, -2.869465e3, -8.237237e2, -8.176940e2],
        ];
        const H50_POS: [f32; 3] = [3013.2314, 50.0004, 1441.2614];

        let projection = ProductionMapCameraProjection::from_camera(
            ProductionMapCamera::new(H2999_M, H2999_POS).expect("far engine camera"),
        )
        .expect("projection from far engine camera");
        assert!((projection.fov_y_degrees() - 44.977).abs() < 0.05);
        assert!((projection.near_z() - 5.0).abs() < 0.05);
        assert!((projection.far_z() - 4000.0).abs() < 5.0);

        for (name, m, pos) in [
            ("h=2999", H2999_M, H2999_POS),
            ("h=499", H500_M, H500_POS),
            ("h=50", H50_M, H50_POS),
        ] {
            let rebuilt = projection
                .camera_from_position(pos)
                .expect("tilt-law camera");
            for r in 0..4 {
                for c in 0..4 {
                    let ours = rebuilt.view_projection_matrix[r][c];
                    let engine = m[r][c];
                    let tol = 0.003_f32.max(engine.abs() * 0.003);
                    assert!(
                        (ours - engine).abs() <= tol,
                        "{name} cell[{r}][{c}]: ours {ours} vs engine {engine}"
                    );
                }
            }
        }
    }

    #[test]
    fn lookat_dz_interpolates_monotone_engine_table() {
        assert!((engine_camera_lookat_dz(10.0) - 37.0370).abs() < 1e-4);
        assert!((engine_camera_lookat_dz(9000.0) - 30.0954).abs() < 1e-4);
        assert!((engine_camera_lookat_dz(499.26) - 30.5819).abs() < 0.005);
        let mut prev = engine_camera_lookat_dz(50.0);
        for h in (60..3000).step_by(37) {
            let dz = engine_camera_lookat_dz(h as f32);
            assert!(dz <= prev + 1e-5, "dz not monotone at h={h}");
            prev = dz;
        }
    }

    #[test]
    fn production_map_camera_rejects_nonfinite_values() {
        let err = ProductionMapCamera::new(
            [
                [f32::NAN, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [0.0, 0.0, 0.0],
        )
        .expect_err("NaN camera matrix must fail closed");
        assert!(err.to_string().contains("non-finite"));

        let err = ProductionMapCamera::new(
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [0.0, f32::INFINITY, 0.0],
        )
        .expect_err("infinite camera position must fail closed");
        assert!(err.to_string().contains("non-finite"));
    }

    #[test]
    fn production_camera_projection_decomposes_captured_row_vector_vp() {
        let camera = ProductionMapCamera::new(
            [
                [1.35876012, 0.0, 0.0, 0.0],
                [0.0, 0.355406135, -0.990354955, -0.989117],
                [0.0, 2.389285, 0.147315308, 0.14713116],
                [-4019.2124, -3629.32373, 581.036255, 585.31],
            ],
            [2958.0, 800.0, 1400.0],
        )
        .expect("captured camera is finite");

        let projection =
            ProductionMapCameraProjection::from_camera(camera).expect("decompose captured VP");

        assert_close_epsilon(projection.fov_y_degrees(), 44.977, 0.01);
        assert_close_epsilon(projection.aspect_ratio(), 16.0 / 9.0, 0.001);
        assert_close_epsilon(projection.near_z(), 5.0, 0.02);
        assert_close_epsilon(projection.far_z(), 4000.0, 3.0);
        assert_close_epsilon(projection.pitch_down_degrees(), 81.54, 0.02);
    }

    #[test]
    fn engine_constants_projection_matches_captured_decomposition() {
        let binary = ProductionMapCameraProjection::from_engine_constants(16.0 / 9.0)
            .expect("binary projection");
        assert_close_epsilon(binary.fov_y_degrees(), 44.977187, 1e-3);
        assert_close_epsilon(binary.fov_y_radians(), 0.785, 1e-5);
        assert_close_epsilon(binary.near_z(), 5.0, 1e-3);
        assert_close_epsilon(binary.far_z(), 4000.0, 5e-2);
        assert_close_epsilon(binary.aspect_ratio(), 16.0 / 9.0, 1e-4);
        let camera = ProductionMapCamera::new(
            [
                [1.358760e0, 0.0, 0.0, 0.0],
                [0.0, 2.423869e-2, -1.001201e0, -9.999496e-1],
                [0.0, 2.415452e0, 1.004690e-2, 1.003434e-2],
                [-3.176648e3, -2.460975e3, 2.987751e3, 2.989016e3],
            ],
            [2337.9019, 2999.0889, 988.7512],
        )
        .expect("captured far camera");
        let captured = ProductionMapCameraProjection::from_camera(camera).expect("decompose");
        assert_close_epsilon(binary.fov_y_degrees(), captured.fov_y_degrees(), 5e-2);
        assert_close_epsilon(binary.aspect_ratio(), captured.aspect_ratio(), 1e-3);
        assert_close_epsilon(binary.near_z(), captured.near_z(), 5e-2);
        assert_close_epsilon(binary.far_z(), captured.far_z(), 5.0);
    }

    #[test]
    fn production_camera_projection_builds_target_centered_camera_and_pick_ray() {
        let base = ProductionMapCamera::new(
            [
                [1.35876012, 0.0, 0.0, 0.0],
                [0.0, 0.355406135, -0.990354955, -0.989117],
                [0.0, 2.389285, 0.147315308, 0.14713116],
                [-4019.2124, -3629.32373, 581.036255, 585.31],
            ],
            [2958.0, 800.0, 1400.0],
        )
        .expect("captured camera is finite");
        let projection =
            ProductionMapCameraProjection::from_camera(base).expect("decompose captured VP");
        let camera = projection
            .camera_for_ground_center(3000.0, 600.0, 350.0)
            .expect("build app-centered camera");

        let picked = camera
            .screen_to_ground_y0(704.0, 256.0, 1408.0, 512.0)
            .expect("screen center ray")
            .expect("ray intersects map plane");

        assert_close_epsilon(picked.0, 3000.0, 0.02);
        assert_close_epsilon(picked.1, 600.0, 0.02);

        let height = projection
            .height_for_screen_zoom(2.5, 1408.0, 50.0, 3000.0)
            .expect("height for app zoom");
        let zoom_camera = projection
            .camera_for_ground_center(3000.0, 600.0, height)
            .expect("build zoom-matched camera");
        let east = zoom_camera
            .screen_to_ground_y0(954.0, 256.0, 1408.0, 512.0)
            .expect("east ray")
            .expect("ray intersects map plane");

        assert_close_epsilon(east.0, 3100.0, 0.03);
        assert_close_epsilon(east.1, 600.0, 0.03);
    }

    #[test]
    fn map_wrap_offsets_keep_center_copy_first() {
        assert_eq!(map_wrap_offsets(100.0, 900.0, 5632.0), vec![0.0]);
        assert_eq!(map_wrap_offsets(-120.0, 480.0, 5632.0), vec![0.0, -5632.0]);
        assert_eq!(map_wrap_offsets(5400.0, 5800.0, 5632.0), vec![0.0, 5632.0]);
    }

    #[test]
    fn map_wrap_offsets_cover_wide_and_invalid_views() {
        assert_eq!(
            map_wrap_offsets(-64.0, 5632.0 + 64.0, 5632.0),
            vec![0.0, -5632.0, 5632.0]
        );
        assert_eq!(
            map_wrap_offsets(100.0, 100.0 + 5633.0, 5632.0),
            vec![0.0, -5632.0, 5632.0]
        );
        assert!(map_wrap_offsets(f32::NAN, 100.0, 5632.0).is_empty());
        assert!(map_wrap_offsets(200.0, 100.0, 5632.0).is_empty());
        assert!(map_wrap_offsets(0.0, 100.0, f32::NAN).is_empty());
    }

    #[test]
    fn hoi4_map_camera_defines_parse_graphics_defines() {
        let defines = hoi4_map_camera_defines_from_source(&minimal_graphics_defines_source())
            .expect("parse the game graphics camera defines");

        assert_close(defines.min_height, 50.0);
        assert_close(defines.max_height, 3000.0);
        assert_close(defines.outside_map_distance_top, 200.0);
        assert_close(defines.outside_map_distance_bottom, 200.0);
        assert_close(defines.zoom_speed, 50.0);
        assert_close(defines.zoom_key_scale, 0.02);
        assert_close(defines.zoom_speed_distance_mult, 6.0);
    }

    fn viewer_camera_fixture() -> ViewerMapCameraState {
        ViewerMapCameraState::new(
            UsedContentKey {
                effective_vfs_manifest_sha256: "a".repeat(64),
                normalized_game_relative_path: BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE.into(),
                winning_layer_ordinal: 0,
                layer_kind: portable_content_manifest::ContentLayerKind::Base,
                layer_id: "base".into(),
                source_sha256: "b".repeat(64),
                payload_sha256: "c".repeat(64),
                payload_bytes: 1,
            },
            Hoi4MapCameraDefines::new(50.0, 3000.0, 200.0, 200.0, 50.0, 0.02, 6.0)
                .expect("camera defines"),
            5632,
            2048,
            1408,
            512,
            50.0,
        )
        .expect("viewer camera")
    }

    #[test]
    fn xmap03_viewer_camera_runs_key_wheel_pan_and_resize_laws() {
        let mut camera = viewer_camera_fixture();
        camera.validate().expect("initial camera validates");

        let initial_target_height = camera.target_eye()[1];
        camera
            .wheel_zoom(704.0, 256.0, 1, 0.016)
            .expect("positive wheel zooms in");
        assert!(camera.target_eye()[1] < initial_target_height);
        camera.advance(0.016).expect("advance camera");

        assert!(camera
            .key_zoom_action(XMAP03_ZOOM_IN_ACTION_ID)
            .expect("runtime zoom-in action"));
        let before_pan = camera.target_look_at();
        camera.pan_screen(120.0, -80.0).expect("projected pan");
        assert_ne!(camera.target_look_at(), before_pan);

        camera.resize_viewport(1920, 1080).expect("resize viewport");
        assert_eq!(
            (camera.viewport_width, camera.viewport_height),
            (1920, 1080)
        );
        camera.validate().expect("mutated camera validates");
    }

    #[test]
    fn xmap03_viewer_camera_rejects_unproven_physical_action_ids() {
        let mut camera = viewer_camera_fixture();
        assert!(camera.key_zoom_action(0x41).is_err());
        assert!(camera.wheel_zoom(10.0, 10.0, 1, f32::NAN).is_err());
        assert!(camera.wheel_zoom(10.0, 10.0, 1, -0.01).is_err());
        camera
            .validate()
            .expect("rejected action leaves state sealed");
    }

    #[test]
    fn xmap03_live_matrix_consumes_interpolated_look_at() {
        let mut camera = viewer_camera_fixture();
        let original = camera
            .production_camera()
            .expect("original production camera");
        let mut look_at = camera.current_look_at();
        look_at[2] += 25.0;
        camera.current_look_at_f32_bits = look_at.map(f32::to_bits);
        camera.reseal().expect("reseal changed look-at");
        let changed = camera
            .production_camera()
            .expect("changed production camera");
        assert_ne!(
            changed.view_projection_matrix,
            original.view_projection_matrix
        );
    }

    #[test]
    fn xmap03_scene_provider_inverse_and_direction_are_live_camera_derived() {
        let camera = viewer_camera_fixture();
        let production = camera.production_camera().expect("production camera");
        let inverse = camera
            .inverse_view_projection_matrix()
            .expect("inverse ViewProjection");
        let mut product = [[0.0f32; 4]; 4];
        for row in 0..4 {
            for column in 0..4 {
                product[row][column] = (0..4)
                    .map(|lane| {
                        production.view_projection_matrix[row][lane] * inverse[lane][column]
                    })
                    .sum();
            }
        }
        for row in 0..4 {
            for column in 0..4 {
                let expected = if row == column { 1.0 } else { 0.0 };
                assert!(
                    (product[row][column] - expected).abs() < 1.0e-3,
                    "matrix product [{row},{column}] = {}",
                    product[row][column]
                );
            }
        }
        let direction = camera
            .current_look_at_direction()
            .expect("camera look direction");
        assert!(direction[1] < 0.0);
        assert!((vec3_len(direction) - 1.0).abs() < 1.0e-5);
    }

    #[test]
    fn xmap03_wheel_ray_subtracts_close_height_map_correction() {
        let mut camera = viewer_camera_fixture();
        let height = 500.0;
        let look_at = [2816.0, 0.0, 1024.0];
        let eye = [
            look_at[0],
            height,
            look_at[2] - engine_camera_lookat_dz(height),
        ];
        camera.current_eye_f32_bits = eye.map(f32::to_bits);
        camera.current_look_at_f32_bits = look_at.map(f32::to_bits);
        camera.target_eye_f32_bits = eye.map(f32::to_bits);
        camera.target_look_at_f32_bits = look_at.map(f32::to_bits);
        camera.reseal().expect("reseal mid-height camera");

        let screen_x = 100.0;
        let screen_y = 200.0;
        let ground = camera
            .production_camera()
            .unwrap()
            .screen_to_ground_y0(
                screen_x,
                screen_y,
                camera.viewport_width as f32,
                camera.viewport_height as f32,
            )
            .unwrap()
            .unwrap();
        let defines = camera.defines().unwrap();
        let h = (height - defines.min_height()) / (defines.max_height() - defines.min_height());
        let corrected_z =
            ground.1 - camera.map_height as f32 * CAMERA_CURSOR_TILT_MAP_HEIGHT_SCALE * (1.0 - h);
        let ray = vec3_normalize([ground.0 - eye[0], -eye[1], corrected_z - eye[2]]).unwrap();
        let base =
            0.016 * f32::from_bits(camera.camera_speed_setting_f32_bits) * defines.zoom_speed();
        let speed = (((h * CAMERA_WHEEL_CURVE_HALF_PI - CAMERA_WHEEL_CURVE_HALF_PI).sin() + 1.0)
            * defines.zoom_speed_distance_mult())
            * base
            + base;

        camera
            .wheel_zoom(screen_x, screen_y, 1, 0.016)
            .expect("wheel update");
        let target = camera.target_eye();
        for component in 0..3 {
            assert!((target[component] - (eye[component] + ray[component] * speed)).abs() < 1.0e-4);
        }
    }

    #[test]
    fn xmap03_low_height_footprint_correction_precedes_margin_clamp() {
        let (top_at_minimum, bottom_at_minimum) =
            low_height_footprint_intersections(100.0, 20.0, 50.0, 50.0, 3000.0);
        assert!((top_at_minimum - 60.0).abs() < 1.0e-5);
        assert!((bottom_at_minimum - 8.0).abs() < 1.0e-5);

        let threshold = 50.0 + (3000.0 - 50.0) * CAMERA_FOOTPRINT_LOW_HEIGHT_FRACTION;
        assert_eq!(
            low_height_footprint_intersections(100.0, 20.0, threshold, 50.0, 3000.0),
            (100.0, 20.0)
        );
    }

    #[test]
    fn gate_screen_zoom_for_height_inverts_height_for_screen_zoom() {
        let projection = ProductionMapCameraProjection::from_camera(
            ProductionMapCamera::new(
                [
                    [1.35876012, 0.0, 0.0, 0.0],
                    [0.0, 0.355406135, -0.990354955, -0.989117],
                    [0.0, 2.389285, 0.147315308, 0.14713116],
                    [-4019.2124, -3629.32373, 581.036255, 585.31],
                ],
                [2958.0, 800.0, 1400.0],
            )
            .expect("captured camera"),
        )
        .expect("decompose captured VP");
        for height in [60.0f32, 250.0, 900.0, 1600.0, 2900.0] {
            let zoom = projection
                .screen_zoom_for_height(height, 1920.0)
                .expect("zoom for height");
            let back = projection
                .height_for_screen_zoom(zoom, 1920.0, 50.0, 3000.0)
                .expect("height for zoom");
            assert!(
                (back - height).abs() / height < 1e-3,
                "round trip {height} -> {zoom} -> {back}"
            );
        }
    }

    #[test]
    fn hoi4_map_camera_defines_load_from_bundle_evidence() {
        let root = temp_test_dir("hoi4_map_camera_defines_from_bundle");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.draw_state = Some(valid_map_frame_draw_state());
            for draw in &mut frame.draws {
                draw.draw_state = Some(valid_map_frame_draw_state());
            }
            frame
        });
        let bundle =
            NativeD3d11TerrainBundle::load(&path).expect("valid native D3D11 terrain bundle");

        let defines =
            hoi4_map_camera_defines_from_bundle(&bundle).expect("parse captured graphics defines");

        assert_close(defines.min_height, 50.0);
        assert_close(defines.max_height, 3000.0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn hoi4_map_camera_defines_fail_closed_on_missing_key() {
        let err = hoi4_map_camera_defines_from_source(
            r#"
                NDefines_Graphics = {
                    NGraphics = {
                        CAMERA_OUTSIDE_MAP_DISTANCE_TOP = 200.0,
                        CAMERA_OUTSIDE_MAP_DISTANCE_BOTTOM = 200.0,
                        CAMERA_ZOOM_SPEED = 50,
                        CAMERA_ZOOM_KEY_SCALE = 0.02,
                    },

                    NFrontend = {
                        CAMERA_MIN_HEIGHT = 50.0,
                        CAMERA_MAX_HEIGHT = 3000.0,
                    },
                }
            "#,
        )
        .expect_err("missing graphics define must fail closed");

        assert!(err.to_string().contains("CAMERA_ZOOM_SPEED_DISTANCE_MULT"));
    }

    #[test]
    fn hoi4_map_camera_defines_fail_closed_on_invalid_height_range() {
        let err = Hoi4MapCameraDefines::new(3000.0, 50.0, 200.0, 200.0, 50.0, 0.02, 6.0)
            .expect_err("inverted camera height range must fail closed");

        assert!(err.to_string().contains("invalid camera height range"));
    }
}
