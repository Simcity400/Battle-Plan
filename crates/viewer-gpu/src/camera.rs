
pub mod frustum;
mod pose;

pub use pose::{Free, Lens, PITCH_LIMIT, Pose};

pub const FOV_Y: f32 = 0.785;
pub const NEAR: f32 = 5.0;
pub const FAR: f32 = 4000.0;
pub const WATER_HEIGHT: f32 = 9.5;

use std::io;

use super::gradient_math::hsv_to_rgb;

pub mod motion;

const FIXED5_SCALE: f32 = 100_000.0;

pub fn fixed5_camera_distance(stored: i64) -> f32 {
    stored as f32 / FIXED5_SCALE
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapIconProjection {
    pub position: [f32; 2],
    pub depth: f32,
}

impl MapIconProjection {
    pub const BEHIND: Self = Self {
        position: [-1.0e9, -1.0e9],
        depth: f32::MAX,
    };
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

const TILT_SCALE: f32 = 0.3;
const TILT_RANGE: [f32; 2] = [5.0, 100.0];

pub fn stored_tilt(value: f32) -> f32 {
    let [low, high] = TILT_RANGE;
    if low > value {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapDefines {
    pub tilt_lead: f32,
    pub heights: [f32; 2],
    pub outside: [f32; 2],
    pub sun: [f32; 3],
    pub moon: [f32; 2],
    pub second_latitude: [f32; 2],
    pub gradient_borders: [f32; 4],
    pub shadows: [f32; 2],
    pub fog: [f32; 2],
    pub tree_fade: [f32; 2],
    pub ambient: [[f32; 3]; 6],
    pub sun_diffuse: [f32; 4],
    pub moon_diffuse: [f32; 4],
    pub raid_icons: [f32; 2],
    pub raid_icons_raid_mode: f32,
    pub sun_specular: f32,
    pub cubemap: f32,
    pub country_names_cutoff: f32,
    pub gradient_field: super::gradient_math::FieldDefines,
    pub border_width: f32,
    pub border_fades: [[f32; 2]; 2],
    pub sprawl_cutoff: f32,
    pub map_objects_cutoff: f32,
    pub railway_cutoff: f32,
    pub railway_cutoff_speed: f32,
}

impl MapDefines {
    pub fn load(defines: &content_reader_defines::defines::Defines, camera_tilt: f32) -> io::Result<Self> {
        let define = |section: &str, key: &str| -> io::Result<f32> {
            let value = defines.number(section, key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!("NDefines.{section}.{key} is outside f32")))
            }
        };
        let number = |key: &str| define("NGraphics", key);
        let color = |key: &str| -> io::Result<[f32; 3]> {
            match defines.numbers("NGraphics", key)?.as_slice() {
                &[h, s, v] => Ok(hsv_linear([h as f32, s as f32, v as f32])),
                _ => Err(invalid(format!(
                    "NDefines.NGraphics.{key} is not three numbers"
                ))),
            }
        };
        let ambient = [
            "AMBIENT_LIGHT_POS_X",
            "AMBIENT_LIGHT_NEG_X",
            "AMBIENT_LIGHT_POS_Y",
            "AMBIENT_LIGHT_NEG_Y",
            "AMBIENT_LIGHT_POS_Z",
            "AMBIENT_LIGHT_NEG_Z",
        ];
        let mut lights = [[0.0; 3]; 6];
        for (slot, key) in lights.iter_mut().zip(ambient) {
            *slot = color(key)?;
        }
        let [sun_r, sun_g, sun_b] = color("SUN_DIFFUSE_COLOR")?;
        let [moon_r, moon_g, moon_b] = color("MOON_DIFFUSE_COLOR")?;
        if !camera_tilt.is_finite() {
            return Err(invalid("settings.txt camera_tilt is not a number"));
        }
        Ok(Self {
            tilt_lead: camera_tilt * TILT_SCALE,
            heights: [
                define("NFrontend", "CAMERA_MIN_HEIGHT")?,
                define("NFrontend", "CAMERA_MAX_HEIGHT")?,
            ],
            outside: [
                number("CAMERA_OUTSIDE_MAP_DISTANCE_TOP")?,
                number("CAMERA_OUTSIDE_MAP_DISTANCE_BOTTOM")?,
            ],
            sun: [
                number("SUN_HEIGHT")?,
                number("SUN_LATITUDE")?,
                number("SUN_HEIGHT_WATER")?,
            ],
            moon: [number("MOON_HEIGHT")?, number("MOON_HEIGHT_WATER")?],
            second_latitude: [
                number("SECOND_SUN_LATITUDE")?,
                number("SECOND_MOON_LATITUDE")?,
            ],
            gradient_borders: [
                number("GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_STRATEGIC_REGIONS")?,
                number("GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_COUNTRY")?,
                number("GRADIENT_BORDERS_OUTLINE_CUTOFF_STRATEGIC_REGIONS")?,
                number("GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY")?,
            ],
            shadows: [
                number("DRAW_SHADOWS_CUTOFF")?,
                number("DRAW_SHADOWS_FADE_LENGTH")?,
            ],
            fog: [number("DRAW_FOW_CUTOFF")?, number("DRAW_FOW_FADE_LENGTH")?],
            tree_fade: [number("TREE_FADE_NEAR")?, number("TREE_FADE_FAR")?],
            ambient: lights,
            sun_diffuse: [sun_r, sun_g, sun_b, number("SUN_INTENSITY")?],
            moon_diffuse: [moon_r, moon_g, moon_b, number("MOON_INTENSITY")?],
            raid_icons: [
                number("RAID_MAP_ICON_DRAW_DISTANCE_MIN")?,
                number("RAID_MAP_ICON_DRAW_DISTANCE_MAX")?,
            ],
            raid_icons_raid_mode: number("RAID_MAP_ICON_MAX_DRAW_DISTANCE_IN_RAID_MAP_MODE")?,
            sun_specular: number("SUN_SPECULAR_INTENSITY")?,
            cubemap: number("CUBEMAP_INTENSITY")?,
            country_names_cutoff: number("DRAW_COUNTRY_NAMES_CUTOFF")?,
            gradient_field: super::gradient_math::FieldDefines::load(defines)?,
            border_width: number("BORDER_WIDTH")?,
            border_fades: [
                [
                    number("PROVINCE_BORDER_FADE_NEAR")?,
                    number("PROVINCE_BORDER_FADE_FAR")?,
                ],
                [
                    number("STATE_BORDER_FADE_NEAR")?,
                    number("STATE_BORDER_FADE_FAR")?,
                ],
            ],
            sprawl_cutoff: number("CITY_SPRAWL_SHRINK_DISTANCE")?,
            map_objects_cutoff: number("DRAW_MAP_OBJECTS_CUTOFF")?,
            railway_cutoff: number("RAILWAY_CAMERA_CUTOFF")?,
            railway_cutoff_speed: number("RAILWAY_CAMERA_CUTOFF_SPEED")?,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test() -> Self {
        Self {
            tilt_lead: 30.0,
            heights: [50.0, 3000.0],
            outside: [200.0, 200.0],
            sun: [600.0, 848.0, 5000.0],
            moon: [600.0, 550.0],
            second_latitude: [100.0, 100.0],
            gradient_borders: [1.0, 0.0, 0.98, 0.973],
            shadows: [400.0, 50.0],
            fog: [400.0, 350.0],
            tree_fade: [250.0, 350.0],
            ambient: [
                [0.2, 0.2, 0.2],
                [0.4, 0.1, 0.6],
                [0.0, 0.0, 0.0],
                [0.35, 0.2, 0.0],
                [0.6, 0.2, 0.924],
                [0.55, 0.1, 0.9],
            ]
            .map(hsv_linear),
            sun_diffuse: [1.0, 1.0, 1.0, 1.0],
            moon_diffuse: {
                let [r, g, b] = hsv_linear([0.58, 0.5, 1.0]);
                [r, g, b, 2.5]
            },
            raid_icons: [10.0, 500.0],
            raid_icons_raid_mode: 1600.0,
            sun_specular: 1.0,
            cubemap: 1.0,
            country_names_cutoff: 260.0,
            gradient_field: super::gradient_math::FieldDefines::TEST,
            border_width: 1.5,
            border_fades: [[200.0, 300.0], [400.0, 500.0]],
            sprawl_cutoff: 220.0,
            map_objects_cutoff: 1100.0,
            railway_cutoff: 300.0,
            railway_cutoff_speed: 3.0,
        }
    }
}

fn hsv_linear(hsv: [f32; 3]) -> [f32; 3] {
    hsv_to_rgb(hsv).map(|channel| channel.powf(2.2))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapSize {
    pub width: u32,
    pub height: u32,
}

impl MapSize {
    pub fn of(map: &content_reader_map::map::MapData) -> Self {
        Self {
            width: map.width,
            height: map.height,
        }
    }

    pub const fn x(self) -> f32 {
        self.width as f32
    }

    pub const fn y(self) -> f32 {
        self.height as f32
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const TEST: Self = Self {
        width: 5632,
        height: 2048,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub center: [f32; 2],
    pub height: f32,
    pub map: MapSize,
    pub defines: MapDefines,
    pub pose: Pose,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub center: [f32; 2],
    pub height: f32,
    pub pose: Pose,
}

impl Placement {
    pub const fn map(center: [f32; 2], height: f32) -> Self {
        Self {
            center,
            height,
            pose: Pose::Map,
        }
    }

    pub fn on(self, map: MapSize, defines: MapDefines) -> Camera {
        Camera {
            center: self.center,
            height: self.height,
            map,
            defines,
            pose: self.pose,
        }
    }
}

struct Basis {
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
}

impl Camera {
    pub fn placement(&self) -> Placement {
        Placement {
            center: self.center,
            height: self.height,
            pose: self.pose,
        }
    }

    pub fn free(&self) -> Option<&Free> {
        match &self.pose {
            Pose::Map => None,
            Pose::Free(free) => Some(free),
        }
    }

    pub fn lens(&self) -> Lens {
        self.free().map_or(Lens::MAP, |free| free.lens)
    }

    pub fn eye(&self) -> [f32; 3] {
        match self.pose {
            Pose::Map => [
                self.center[0],
                self.height,
                self.center[1] - self.defines.tilt_lead,
            ],
            Pose::Free(_) => [self.center[0], self.height, self.center[1]],
        }
    }

    pub fn turned_from(&self, other: &Camera) -> Option<(f32, f32)> {
        let turn = match (&self.pose, &other.pose) {
            (pose, other) if pose == other => Some(0.0),
            (Pose::Free(free), Pose::Free(other)) if free.lens == other.lens => {
                let moved: f32 = free
                    .axes()
                    .iter()
                    .zip(other.axes())
                    .map(|(a, b)| subtract(*a, b))
                    .map(|d| dot(d, d))
                    .sum();
                Some((moved * 0.5).sqrt().max(f32::MIN_POSITIVE))
            }
            _ => None,
        };
        let same = self.height == other.height && self.map == other.map && self.defines == other.defines;
        let [dx, dz] = [other.center[0] - self.center[0], other.center[1] - self.center[1]];
        turn.filter(|_| same).map(|turn| ((dx * dx + dz * dz).sqrt(), turn))
    }

    pub fn look_at(&self) -> [f32; 3] {
        match &self.pose {
            Pose::Map => [self.center[0], WATER_HEIGHT, self.center[1]],
            Pose::Free(free) => {
                let [eye, forward] = [self.eye(), free.forward()];
                std::array::from_fn(|axis| eye[axis] + forward[axis])
            }
        }
    }

    pub fn look_direction(&self) -> [f32; 3] {
        match &self.pose {
            Pose::Map => normalize(subtract(self.look_at(), self.eye())),
            Pose::Free(free) => free.forward(),
        }
    }

    fn basis(&self) -> Basis {
        if let Pose::Free(free) = &self.pose {
            let [right, up, forward] = free.axes();
            return Basis { right, up, forward };
        }
        let forward = self.look_direction();
        let right = normalize(cross([0.0, 1.0, 0.0], forward));
        Basis {
            up: cross(forward, right),
            right,
            forward,
        }
    }

    pub fn world_text_axes(&self) -> ([f32; 3], [f32; 3]) {
        let Basis { right, up, .. } = self.basis();
        (right, normalize(up))
    }

    pub fn axes(&self) -> [[f32; 3]; 3] {
        let Basis { right, up, forward } = self.basis();
        [right, up, forward]
    }

    pub fn view_projection(&self, viewport: [u32; 2]) -> [[f32; 4]; 4] {
        multiply(self.view(), projection(self.lens(), aspect(viewport)))
    }

    pub fn projection(&self, viewport: [u32; 2]) -> [[f32; 4]; 4] {
        projection(self.lens(), aspect(viewport))
    }

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

    pub fn project_map_icon(
        &self,
        world: [f32; 3],
        viewport: [u32; 2],
        extent: [i32; 2],
    ) -> Option<MapIconProjection> {
        let width = self.map.x() as i32;
        let (step, half) = (width as f32, (width / 2) as f32);
        let mut x = world[0];
        while (self.center[0] - x).abs() > half && step > 0.0 {
            x += if self.center[0] - x < 0.0 {
                -step
            } else {
                step
            };
        }
        let matrix = self.view_projection(viewport);
        let world = [x, world[1], world[2]];
        if self.free().is_some() {
            let w = world[0] * matrix[0][3]
                + world[1] * matrix[1][3]
                + world[2] * matrix[2][3]
                + matrix[3][3];
            if w.is_nan() || w <= self.lens().near * 0.5 {
                return None;
            }
        }
        Some(Self::project_map_icon_matrix(&matrix, world, extent))
    }

    pub fn project_map_icon_matrix(
        matrix: &[[f32; 4]; 4],
        world: [f32; 3],
        extent: [i32; 2],
    ) -> MapIconProjection {
        let clip = |column: usize| {
            world[1] * matrix[1][column]
                + world[0] * matrix[0][column]
                + world[2] * matrix[2][column]
                + matrix[3][column]
        };
        let w = clip(3);
        MapIconProjection {
            position: [
                (clip(0) / w * 0.5 + 0.5) * extent[0] as f32,
                (clip(1) / w * -0.5 + 0.5) * extent[1] as f32,
            ],
            depth: Self::map_icon_depth(matrix, world),
        }
    }

    pub fn map_icon_depth(matrix: &[[f32; 4]; 4], world: [f32; 3]) -> f32 {
        let clip = |column: usize| {
            world[1] * matrix[1][column]
                + world[0] * matrix[0][column]
                + world[2] * matrix[2][column]
                + matrix[3][column]
        };
        clip(2) / clip(3)
    }

    pub fn project_wrapped(&self, world: [f32; 3], viewport: [u32; 2]) -> Option<[f32; 2]> {
        let offset = self.wrap_offset(world, viewport);
        self.project([world[0] + offset, world[1], world[2]], viewport)
    }

    pub fn wrap_offset(&self, world: [f32; 3], viewport: [u32; 2]) -> f32 {
        let middle = viewport[0].max(1) as f32 * 0.5;
        let width = self.map.x();
        [0.0, -width, width]
            .into_iter()
            .filter_map(|offset| {
                let screen = self.project([world[0] + offset, world[1], world[2]], viewport)?;
                Some((offset, (screen[0] - middle).abs()))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0.0, |(offset, _)| offset)
    }

    pub fn unproject_ground(&self, screen: [f32; 2], viewport: [u32; 2]) -> Option<[f32; 2]> {
        let direction = self.ray(screen, viewport);
        if direction[1] > -1e-6 {
            return None;
        }
        let eye = self.eye();
        let t = (WATER_HEIGHT - eye[1]) / direction[1];
        Some([eye[0] + t * direction[0], eye[2] + t * direction[2]])
    }

    pub fn ray(&self, screen: [f32; 2], viewport: [u32; 2]) -> [f32; 3] {
        let width = viewport[0].max(1) as f32;
        let height = viewport[1].max(1) as f32;
        let ndc_x = screen[0] / width * 2.0 - 1.0;
        let ndc_y = 1.0 - screen[1] / height * 2.0;
        let y_scale = 1.0 / (self.lens().fov_y * 0.5).tan();
        let view = [ndc_x * aspect(viewport) / y_scale, ndc_y / y_scale, 1.0];
        let Basis { right, up, forward } = self.basis();
        [
            right[0] * view[0] + up[0] * view[1] + forward[0] * view[2],
            right[1] * view[0] + up[1] * view[1] + forward[1] * view[2],
            right[2] * view[0] + up[2] * view[1] + forward[2] * view[2],
        ]
    }

    pub fn pick_ray(&self, screen: [f32; 2], viewport: [u32; 2]) -> [f32; 3] {
        let [x, y] = screen.map(|value| value as i32 as f32);
        let x = (x + x) / viewport[0].max(1) as f32 - 1.0;
        let y = 1.0 - (y + y) / viewport[1].max(1) as f32;
        self.ray_at_ndc([x, y], viewport)
    }

    pub fn pan_direction(&self, screen: [f32; 2], viewport: [u32; 2]) -> [f32; 3] {
        let ray = self.pick_ray(screen, viewport);
        let Basis { right, up, .. } = self.basis();
        let across = right[0] * ray[0] + right[1] * ray[1] + right[2] * ray[2];
        let rise = up[0] * ray[0] + up[1] * ray[1] + up[2] * ray[2];
        [
            across * right[0] + rise * up[0],
            0.0,
            across * right[2] + rise * up[2],
        ]
    }

    pub fn ray_at_ndc(&self, [x, y]: [f32; 2], viewport: [u32; 2]) -> [f32; 3] {
        let lens = self.lens();
        let tangent = (lens.fov_y * 0.5).tan();
        let reciprocal_w = 1.0 / (1.0 / lens.near);
        let view = [
            x * (tangent * aspect(viewport)) * reciprocal_w,
            y * tangent * reciprocal_w,
            reciprocal_w,
        ];
        let Basis { right, up, forward } = self.basis();
        let up = normalize(up);
        if self.free().is_some() {
            return normalize(std::array::from_fn(|axis| {
                view[1] * up[axis] + view[0] * right[axis] + view[2] * forward[axis]
            }));
        }
        let eye = self.eye();
        normalize(std::array::from_fn(|axis| {
            (view[1] * up[axis] + view[0] * right[axis] + view[2] * forward[axis] + eye[axis])
                - eye[axis]
        }))
    }

    pub fn clamp_center(&mut self, viewport: [u32; 2]) {
        if self.free().is_some() {
            return;
        }
        let Some([_, low, _, high]) = self.visible_ground(viewport, 0.0) else {
            return;
        };
        let [top, bottom_distance] = self.defines.outside;
        let bottom = low + bottom_distance;
        let over_top = high - (self.map.y() + top);
        self.center[1] -= if high - low > self.map.y() + top + bottom_distance {
            (bottom + over_top) * 0.5
        } else if over_top > 0.0 {
            over_top
        } else if bottom < 0.0 {
            bottom
        } else {
            return;
        };
    }

    pub fn pan(&mut self, from: [f32; 2], to: [f32; 2], viewport: [u32; 2]) {
        let (Some(from), Some(to)) = (
            self.unproject_ground(from, viewport),
            self.unproject_ground(to, viewport),
        ) else {
            return;
        };
        self.center[0] += from[0] - to[0];
        self.center[1] += from[1] - to[1];
        self.clamp_center(viewport);
    }

    pub fn fit(map: MapSize, defines: MapDefines, viewport: [u32; 2]) -> Self {
        let center = [map.x() * 0.5, map.y() * 0.5];
        let covers = |height: f32| {
            let camera = Camera {
                center,
                height,
                map,
                defines,
                pose: Pose::Map,
            };
            match camera.visible_ground(viewport, 0.0) {
                Some(rect) => {
                    rect[0] <= 0.0 && rect[1] <= 0.0 && rect[2] >= map.x() && rect[3] >= map.y()
                }
                None => false,
            }
        };
        let [mut low, mut high] = defines.heights;
        if !covers(high) {
            return Self {
                center,
                height: high,
                map,
                defines,
                pose: Pose::Map,
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
            map,
            defines,
            pose: Pose::Map,
        }
    }

    pub fn visible_ground(&self, viewport: [u32; 2], margin: f32) -> Option<[f32; 4]> {
        if self.free().is_some() {
            let range = self.object_range();
            let eye = self.eye();
            let rect = self
                .reach(viewport, range, [0.0, frustum::OBJECT_TOP])
                .unwrap_or([eye[0], eye[2], eye[0], eye[2]]);
            return Some([
                rect[0] - margin,
                rect[1] - margin,
                rect[2] + margin,
                rect[3] + margin,
            ]);
        }
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

    pub fn held_ground(&self, viewport: [u32; 2], margin: f32, slack: Slack) -> Option<[f32; 4]> {
        if slack.turn == 0.0 || self.free().is_none() {
            return self.visible_ground(viewport, margin + slack.pad);
        }
        let eye = self.eye();
        let mut points = frustum::in_band(
            &frustum::turned_corners(self, viewport, self.lens().near, self.object_range(), slack.turn)?,
            0.0,
            frustum::OBJECT_TOP,
        );
        points.push(eye);
        let rect = frustum::bounds(&points).expect("the eye is a point");
        let margin = margin + slack.pad;
        Some([rect[0] - margin, rect[1] - margin, rect[2] + margin, rect[3] + margin])
    }

    pub fn object_range(&self) -> f32 {
        self.defines.map_objects_cutoff.min(self.lens().far)
    }

    pub fn icon_distance(&self, anchor: [f32; 3]) -> f32 {
        match self.pose {
            Pose::Map => self.height,
            Pose::Free(_) => {
                let eye = self.eye();
                let width = self.map.x();
                let mut dx = anchor[0] - eye[0];
                if width > 0.0 {
                    dx -= width * (dx / width).round();
                }
                let (dy, dz) = (anchor[1] - eye[1], anchor[2] - eye[2]);
                (dx * dx + dy * dy + dz * dz).sqrt()
            }
        }
    }

    pub fn reach(&self, viewport: [u32; 2], range: f32, band: [f32; 2]) -> Option<[f32; 4]> {
        frustum::bounds(&self.reach_points(viewport, range, band))
    }

    pub fn reach_points(&self, viewport: [u32; 2], range: f32, band: [f32; 2]) -> Vec<[f32; 3]> {
        let near = self.lens().near;
        let corners = frustum::corners(self, viewport, near, range.max(near));
        frustum::in_band(&corners, band[0], band[1])
    }

    pub fn frustum_planes(&self, viewport: [u32; 2]) -> [[f32; 4]; 6] {
        frustum::planes(&self.view_projection(viewport))
    }

    pub fn hold_free(&mut self, floor: &dyn Fn(f32, f32) -> f32, clearance: f32) {
        if self.free().is_none() {
            return;
        }
        let width = self.map.x();
        if width > 0.0 {
            self.center[0] = self.center[0].rem_euclid(width);
        }
        self.center[1] = self.center[1].clamp(0.0, self.map.y());
        let low = floor(self.center[0], self.center[1]).max(WATER_HEIGHT) + clearance;
        let high = self.defines.heights[1].max(low);
        self.height = self.height.clamp(low, high);
    }

    pub fn validate(self) -> std::io::Result<()> {
        let pose = match &self.pose {
            Pose::Map => true,
            Pose::Free(free) => free.is_finite() && free.lens.is_valid(),
        };
        if self.center.iter().all(|value| value.is_finite())
            && self.height.is_finite()
            && self.height > WATER_HEIGHT
            && pose
        {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "camera centre must be finite, height must exceed the water plane {WATER_HEIGHT} \
                     and a free pose needs a finite orientation and a usable lens"
                ),
            ))
        }
    }
}

fn aspect(viewport: [u32; 2]) -> f32 {
    viewport[0].max(1) as f32 / viewport[1].max(1) as f32
}

fn projection(lens: Lens, aspect: f32) -> [[f32; 4]; 4] {
    let Lens { fov_y, near, far } = lens;
    let y_scale = 1.0 / (fov_y * 0.5).tan();
    [
        [y_scale / aspect, 0.0, 0.0, 0.0],
        [0.0, y_scale, 0.0, 0.0],
        [0.0, 0.0, far / (far - near), 1.0],
        [0.0, 0.0, -near * far / (far - near), 0.0],
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

pub const PAD: f32 = 8.0;

pub const TURN: f32 = 0.05;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slack {
    pub pad: f32,
    pub turn: f32,
}

impl Slack {
    pub const NONE: Self = Self { pad: 0.0, turn: 0.0 };

    pub const PAN: Self = Self { pad: PAD, turn: 0.0 };

    pub const TURNING: Self = Self { pad: PAD, turn: TURN };

    pub fn at(self, distance: f32) -> f32 {
        self.pad + self.turn * distance
    }

    fn covers(self, (travel, turn): (f32, f32)) -> bool {
        travel <= self.pad && turn <= self.turn
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hold {
    seen: Option<(Camera, [u32; 2])>,
    held: Option<(Camera, [u32; 2], Slack)>,
}

impl Hold {
    pub fn at(&mut self, camera: Camera, viewport: [u32; 2]) -> (Camera, Slack) {
        let last = self.seen.replace((camera, viewport));
        if last.is_none_or(|last| last == (camera, viewport)) {
            self.held = Some((camera, viewport, Slack::NONE));
            return (camera, Slack::NONE);
        }
        if let Some((held, held_viewport, slack)) = self.held
            && slack != Slack::NONE
            && held_viewport == viewport
            && held.turned_from(&camera).is_some_and(|moved| slack.covers(moved))
        {
            return (held, slack);
        }
        let turned = last
            .filter(|(_, last_viewport)| *last_viewport == viewport)
            .and_then(|(last, _)| last.turned_from(&camera))
            .is_some_and(|(_, turn)| turn > 0.0);
        let slack = if turned && frustum::turned_corners(&camera, viewport, 1.0, 1.0, TURN).is_some() {
            Slack::TURNING
        } else {
            Slack::PAN
        };
        self.held = Some((camera, viewport, slack));
        (camera, slack)
    }
}

#[cfg(test)]
mod tests;
