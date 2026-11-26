
use bytemuck::{Pod, Zeroable};

use super::camera::{Camera, MapDefines, MapSize};

fn seasonal_direction(hour: i32) -> [f32; 3] {
    let scale = replay_data::day_night::SCALE as f32;
    let [x, y, z] = replay_data::day_night::seasonal_direction(hour).map(|v| v as f32 / scale);
    let length = (x * x + y * y + z * z).sqrt();
    [x / length, y / length, z / length]
}

pub fn shadow_fade_factor(defines: &MapDefines, camera_height: f32) -> f32 {
    let [cutoff, fade] = defines.shadows;
    if camera_height <= cutoff - fade {
        1.0
    } else {
        ((cutoff - camera_height) / fade).max(0.0)
    }
}

pub fn fow_fade_factor(defines: &MapDefines, camera_height: f32, fog: bool) -> f32 {
    let [cutoff, length] = defines.fog;
    if !fog {
        0.0
    } else if camera_height <= cutoff - length {
        1.0
    } else {
        let fade = ((cutoff - camera_height) / length).max(0.0);
        fade * fade
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayNight {
    pub hour_fraction: f32,
    pub sun_direction: [f32; 3],
    pub strength: f32,
}

impl DayNight {
    pub fn blending_to(self, target: Self) -> bool {
        const LIGHT_TOLERANCE: f32 = 0.00001;
        self.hour_fraction != target.hour_fraction
            || (self.strength - target.strength).abs() > LIGHT_TOLERANCE
            || self
                .sun_direction
                .iter()
                .zip(target.sun_direction)
                .any(|(a, b)| (a - b).abs() > LIGHT_TOLERANCE)
    }

    pub fn at_hour(hour: i32) -> Self {
        Self {
            hour_fraction: hour_fraction(hour),
            sun_direction: seasonal_direction(hour),
            strength: 1.0,
        }
    }

    pub fn with_cycle(mut self, enabled: bool) -> Self {
        self.strength = if enabled { 1.0 } else { 0.0 };
        self
    }

    pub fn hour_sun_dir(&self) -> [f32; 4] {
        [
            self.hour_fraction,
            self.sun_direction[0],
            self.sun_direction[1],
            self.sun_direction[2],
        ]
    }

    pub fn sun_position(&self, map: MapSize, defines: &MapDefines) -> [f32; 3] {
        let midpoint = map.x() * 0.5;
        let moving = (1.0 - self.hour_fraction) * map.x();
        [
            (moving - midpoint) * self.strength + midpoint,
            defines.sun[0],
            defines.sun[1],
        ]
    }

    pub fn moon_position(&self, map: MapSize, defines: &MapDefines) -> [f32; 3] {
        [
            wrap_x(map, self.sun_position(map, defines)[0] + map.x() * 0.5),
            defines.moon[0],
            map.y() - defines.sun[0],
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameSpeeds([f64; 5]);

impl GameSpeeds {
    const FASTEST: usize = 4;

    pub fn load(defines: &content_reader_defines::defines::Defines) -> Result<Self, String> {
        let read = defines
            .numbers("NGame", "GAME_SPEED_SECONDS")
            .map_err(|error| error.to_string())?;
        let mut seconds = [0.0; 5];
        for (slot, value) in seconds.iter_mut().zip(read) {
            *slot = f64::from(value as f32);
        }
        Ok(Self(seconds))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test() -> Self {
        Self([2.0_f32, 0.5, 0.2, 0.1, 0.0].map(f64::from))
    }

    pub fn level(&self, hours_per_second: f64) -> usize {
        self.0
            .iter()
            .position(|&seconds| seconds <= 0.0 || seconds * hours_per_second <= 1.0)
            .unwrap_or(Self::FASTEST)
    }

    pub fn shows_cycle(&self, playing: bool, hours_per_second: f64) -> bool {
        !playing || self.level(hours_per_second) < Self::FASTEST
    }
}

#[derive(Default)]
pub struct DayNightPlayback {
    previous: Option<(i32, DayNight, f64)>,
    changed_at: f64,
}

impl DayNightPlayback {
    pub fn update(&mut self, hour: i32, target: DayNight, time: f64, interval: f64) -> DayNight {
        let Some((previous_hour, mut shown, previous_time)) = self.previous else {
            self.previous = Some((hour, target, time));
            self.changed_at = time;
            return target;
        };
        if hour != previous_hour {
            self.changed_at = time;
        }
        let elapsed = (time - previous_time).max(0.0);
        let remaining = self.changed_at + interval - time;
        let blend = if remaining > 0.0 {
            (elapsed / remaining).clamp(0.0, 1.0) as f32
        } else {
            1.0
        };
        if (0..24).contains(&(i64::from(hour) - i64::from(previous_hour))) {
            let delta = (target.hour_fraction - shown.hour_fraction).rem_euclid(1.0);
            shown.hour_fraction = (shown.hour_fraction + delta * blend).rem_euclid(1.0);
        } else {
            shown.hour_fraction = target.hour_fraction;
        }
        let blend = (12.0 * elapsed as f32).clamp(0.001, 1.0);
        for axis in 0..3 {
            shown.sun_direction[axis] +=
                (target.sun_direction[axis] - shown.sun_direction[axis]) * blend;
        }
        let length = shown
            .sun_direction
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        shown.sun_direction = shown.sun_direction.map(|value| value / length);
        shown.strength += (target.strength - shown.strength) * blend;
        self.previous = Some((hour, shown, time));
        shown
    }
}

pub fn hour_fraction(hour: i32) -> f32 {
    (hour.rem_euclid(24)) as f32 / 24.0
}

fn wrap_x(map: MapSize, x: f32) -> f32 {
    x - map.x() * (x / map.x()).floor()
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FrameConstants {
    pub view_projection: [[f32; 4]; 4],
    pub inv_view_projection: [[f32; 4]; 4],
    pub gb_cam_dist_override_outline_cutoff: [f32; 4],
    pub virtual_sun_pos: [f32; 4],
    pub virtual_moon_pos: [f32; 4],
    pub second_virtual_sun_pos: [f32; 4],
    pub second_virtual_moon_pos: [f32; 4],
    pub cam_pos_hdr_range: [f32; 4],
    pub cam_look_at_dir_global_time: [f32; 4],
    pub fow_opacity_time_snow_mud_day_night: [f32; 4],
    pub day_night_hour_sun_dir: [f32; 4],
    pub ambient_pos_x_shadow_fade: [f32; 4],
    pub ambient_neg_x_fow_fade: [f32; 4],
    pub ambient_pos_y_min_mesh_alpha: [f32; 4],
    pub ambient_neg_y_neg_fog: [f32; 4],
    pub ambient_pos_z: [f32; 4],
    pub ambient_neg_z_cubemap: [f32; 4],
    pub sun_diffuse_intensity: [f32; 4],
    pub moon_diffuse_intensity: [f32; 4],
    pub gb_texture_height_sun_specular: [f32; 4],
}

impl FrameConstants {
    pub fn with_channels(mut self, channels: [Option<[f32; 2]>; 2]) -> Self {
        for (index, channel) in channels.into_iter().enumerate() {
            if let Some([override_distance, cutoff]) = channel {
                self.gb_cam_dist_override_outline_cutoff[index] = override_distance;
                self.gb_cam_dist_override_outline_cutoff[index + 2] = cutoff;
            }
        }
        self
    }

    pub fn with_snow_mud_fade(mut self, fade: f32) -> Self {
        self.fow_opacity_time_snow_mud_day_night[2] = fade;
        self
    }

    pub fn try_new(
        camera: Camera,
        viewport: [u32; 2],
        day_night: DayNight,
        global_time: f32,
        fog: bool,
    ) -> Option<Self> {
        let inv_view_projection = camera.inverse(viewport)?;
        if !inv_view_projection
            .iter()
            .flatten()
            .all(|value| value.is_finite())
        {
            return None;
        }
        let defines = &camera.defines;
        let sun = day_night.sun_position(camera.map, defines);
        let moon = day_night.moon_position(camera.map, defines);
        let [sun_height, _, sun_water] = defines.sun;
        let [moon_height, moon_water] = defines.moon;
        let ambient = |index: usize, w: f32| {
            let [r, g, b] = defines.ambient[index];
            [r, g, b, w]
        };
        let eye = camera.eye();
        let look = camera.look_direction();
        let view_projection = camera.view_projection(viewport);
        Some(Self {
            view_projection,
            inv_view_projection,
            gb_cam_dist_override_outline_cutoff: defines.gradient_borders,
            virtual_sun_pos: [sun[0], sun[1], sun[2], sun_water],
            virtual_moon_pos: [moon[0], moon[1], moon[2], moon_water],
            second_virtual_sun_pos: [
                wrap_x(camera.map, sun[0] + camera.map.x() * 0.5),
                sun_height,
                defines.second_latitude[0],
                sun_water,
            ],
            second_virtual_moon_pos: [
                wrap_x(camera.map, moon[0] + camera.map.x() * 0.5),
                moon_height,
                defines.second_latitude[1],
                moon_water,
            ],
            cam_pos_hdr_range: [eye[0], eye[1], eye[2], 1.0],
            cam_look_at_dir_global_time: [look[0], look[1], look[2], global_time],
            fow_opacity_time_snow_mud_day_night: [
                f32::from(u8::from(fog)),
                global_time,
                1.0,
                day_night.strength,
            ],
            day_night_hour_sun_dir: day_night.hour_sun_dir(),
            ambient_pos_x_shadow_fade: ambient(0, shadow_fade_factor(defines, eye[1])),
            ambient_neg_x_fow_fade: ambient(1, fow_fade_factor(defines, eye[1], fog)),
            ambient_pos_y_min_mesh_alpha: ambient(2, 0.0),
            ambient_neg_y_neg_fog: ambient(3, 1.0),
            ambient_pos_z: ambient(4, 0.0),
            ambient_neg_z_cubemap: ambient(5, defines.cubemap),
            sun_diffuse_intensity: defines.sun_diffuse,
            moon_diffuse_intensity: defines.moon_diffuse,
            gb_texture_height_sun_specular: [
                super::textures::gradient_layer_height(camera.map),
                defines.sun_specular,
                0.0,
                0.0,
            ],
        })
    }
}

#[cfg(test)]
mod tests;
