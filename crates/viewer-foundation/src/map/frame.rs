//! `ConstantBuffer(0, 0)` from `standardfuncsgfx.fxh`, field for field.
//!
//! The game's HLSL constant buffer packs `float3`/`float` pairs into one
//! 16-byte register and starts a `float3` that does not fit in the remainder of
//! a register on the next one. WGSL's uniform layout rules are the same, so the
//! struct below mirrors the declaration with every register spelled out as a
//! `[f32; 4]`. The declared order and the register each field lands in:
//!
//! | offset | field |
//! |---|---|
//! | 0 | `ViewProjectionMatrix` |
//! | 64 | `InvViewProjMatrix` |
//! | 128 | `vGBCamDistOverride_GBOutlineCutoff` |
//! | 144 | `vVirtualSunPos` |
//! | 160 | `vVirtualMoonPos` |
//! | 176 | `vSecondVirtualSunPos` |
//! | 192 | `vSecondVirtualMoonPos` |
//! | 208 | `vCamPos` + `HdrRange` |
//! | 224 | `vCamLookAtDir` + `vGlobalTime` |
//! | 240 | `vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed` |
//! | 256 | `DayNight_Hour_SunDir` |
//! | 272 | `AmbientPosX` + `ShadowFadeFactor` |
//! | 288 | `AmbientNegX` + `FOWFadeFactor` |
//! | 304 | `AmbientPosY` + `MinMeshAlpha` |
//! | 320 | `AmbientNegY` + `NegFogMultiplier` |
//! | 336 | `AmbientPosZ` (pad) |
//! | 352 | `AmbientNegZ` + `CubemapIntensity` |
//! | 368 | `SunDiffuseIntensity` |
//! | 384 | `MoonDiffuseIntensity` |
//! | 400 | `GB_TextureHeight` + `SunSpecularIntensity` (pad) |
//!

use bytemuck::{Pod, Zeroable};

use super::camera::{Camera, MapDefines, MapSize};

/// Settled wind direction for a replay hour.
///
/// The hourly weather update rebuilds the target each hour; weather initialization rebuilds it too.
/// There is no simulation smoothing or save-history dependency. The renderer alone does
/// `normalize(previous + (raw / 100000 - previous) * alpha)`, with `alpha = clamp(12 *
/// filtered_frame_seconds, 0.001, 1)` each frame. We start at the settled target, so seeks do not
/// introduce a synthetic load transient. Unrecorded renderer transients and disabled-weather stale
/// targets remain UNVERIFIED; the exact fields and conditional error bounds are in [archived
/// rendering
/// notes](#)
/// section 3.1. The virtual lighting sun uses only the hour, not this direction. The exact
/// simulation target is [`replay_data::day_night::seasonal_direction`], the signed fixed5 words the
/// weather manager holds.
fn seasonal_direction(hour: i32) -> [f32; 3] {
    let scale = replay_data::day_night::SCALE as f32;
    let [x, y, z] = replay_data::day_night::seasonal_direction(hour).map(|v| v as f32 / scale);
    let length = (x * x + y * y + z * z).sqrt();
    [x / length, y / length, z / length]
}

/// Shadow fade is one below the configured fade band, then decreases linearly
/// toward zero at the shadow cutoff. Above the cutoff the sampled shadow term
/// is fully lit, regardless of the buffer's contents.
pub fn shadow_fade_factor(defines: &MapDefines, camera_height: f32) -> f32 {
    let [cutoff, fade] = defines.shadows;
    if camera_height <= cutoff - fade {
        1.0
    } else {
        ((cutoff - camera_height) / fade).max(0.0)
    }
}

/// Fog fade is zero when the frame has no fog input. Otherwise it is one below
/// the configured fade band, then follows the square of the nonnegative linear
/// fade toward the fog cutoff. Controller and owner views use political-mode
/// fog gating. Missing historical inputs and viewpoint mismatches leave a clear
/// intel map, so there is no fog contribution to scale.
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

/// Every value the frame constants need that is not the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayNight {
    /// `DayNight_Hour_SunDir.x`: `(hour mod 24) / 24` on the raw replay hour.
    pub hour_fraction: f32,
    /// Settled, normalized seasonal direction from the weather simulation.
    pub sun_direction: [f32; 3],
    /// `vFoWOpacity_..._MaxGameSpeed.w`: settled display strength, 0 when the
    /// viewer disables the cycle, otherwise 1.
    pub strength: f32,
}

impl DayNight {
    /// The day/night state at a replay hour.
    ///
    /// the engine's `(hour - 43800000) mod 24` is the raw hour
    /// modulo 24 because `43800000 mod 24 == 0`, and it is one less than the
    /// hour the interface shows. That offset is the engine's, not a defect.
    pub fn at_hour(hour: i32) -> Self {
        Self {
            hour_fraction: hour_fraction(hour),
            sun_direction: seasonal_direction(hour),
            strength: 1.0,
        }
    }

    /// Changes display lighting without changing the simulation hour or season.
    pub fn with_cycle(mut self, enabled: bool) -> Self {
        self.strength = if enabled { 1.0 } else { 0.0 };
        self
    }

    /// `DayNight_Hour_SunDir`: the hour fraction and the night direction.
    ///
    /// The direction's `x` is zero because `CalcGlobeNormal` already bakes the
    /// hour rotation into the globe normal.
    pub fn hour_sun_dir(&self) -> [f32; 4] {
        [
            self.hour_fraction,
            self.sun_direction[0],
            self.sun_direction[1],
            self.sun_direction[2],
        ]
    }

    /// `vVirtualSunPos.xyz`.
    pub fn sun_position(&self, map: MapSize, defines: &MapDefines) -> [f32; 3] {
        // Blend virtual sun X toward the map midpoint as strength
        // falls to zero; the moon and shadow direction use this same position.
        let midpoint = map.x() * 0.5;
        let moving = (1.0 - self.hour_fraction) * map.x();
        [
            (moving - midpoint) * self.strength + midpoint,
            defines.sun[0],
            defines.sun[1],
        ]
    }

    /// `vVirtualMoonPos.xyz`.
    ///
    /// Moon z is map height minus the sun position's vertical component.
    pub fn moon_position(&self, map: MapSize, defines: &MapDefines) -> [f32; 3] {
        [
            wrap_x(map, self.sun_position(map, defines)[0] + map.x() * 0.5),
            defines.moon[0],
            map.y() - defines.sun[0],
        ]
    }
}

/// `(hour mod 24) / 24`, handling negative hours the way a modulo should.
pub fn hour_fraction(hour: i32) -> f32 {
    (hour.rem_euclid(24)) as f32 / 24.0
}

fn wrap_x(map: MapSize, x: f32) -> f32 {
    x - map.x() * (x / map.x()).floor()
}

/// The GPU mirror of `ConstantBuffer(0, 0)`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FrameConstants {
    /// Row-major rows, which WGSL reads as the column-vector transpose.
    pub view_projection: [[f32; 4]; 4],
    pub inv_view_projection: [[f32; 4]; 4],
    pub gb_cam_dist_override_outline_cutoff: [f32; 4],
    pub virtual_sun_pos: [f32; 4],
    pub virtual_moon_pos: [f32; 4],
    pub second_virtual_sun_pos: [f32; 4],
    pub second_virtual_moon_pos: [f32; 4],
    /// `vCamPos.xyz`, `HdrRange`.
    pub cam_pos_hdr_range: [f32; 4],
    /// `vCamLookAtDir.xyz`, `vGlobalTime`.
    pub cam_look_at_dir_global_time: [f32; 4],
    pub fow_opacity_time_snow_mud_day_night: [f32; 4],
    pub day_night_hour_sun_dir: [f32; 4],
    /// `AmbientPosX.rgb`, `ShadowFadeFactor`.
    pub ambient_pos_x_shadow_fade: [f32; 4],
    /// `AmbientNegX.rgb`, `FOWFadeFactor`.
    pub ambient_neg_x_fow_fade: [f32; 4],
    /// `AmbientPosY.rgb`, `MinMeshAlpha`.
    pub ambient_pos_y_min_mesh_alpha: [f32; 4],
    /// `AmbientNegY.rgb`, `NegFogMultiplier`.
    pub ambient_neg_y_neg_fog: [f32; 4],
    /// `AmbientPosZ.rgb` and the register padding HLSL leaves behind.
    pub ambient_pos_z: [f32; 4],
    /// `AmbientNegZ.rgb`, `CubemapIntensity`.
    pub ambient_neg_z_cubemap: [f32; 4],
    pub sun_diffuse_intensity: [f32; 4],
    pub moon_diffuse_intensity: [f32; 4],
    /// `GB_TextureHeight`, `SunSpecularIntensity` and the trailing padding.
    pub gb_texture_height_sun_specular: [f32; 4],
}

impl FrameConstants {
    /// A map mode's camera-distance override and outline cutoff for each channel; `None` keeps
    /// the content's strategic or country defines.
    pub fn with_channels(mut self, channels: [Option<[f32; 2]>; 2]) -> Self {
        for (index, channel) in channels.into_iter().enumerate() {
            if let Some([override_distance, cutoff]) = channel {
                self.gb_cam_dist_override_outline_cutoff[index] = override_distance;
                self.gb_cam_dist_override_outline_cutoff[index + 2] = cutoff;
            }
        }
        self
    }

    /// Apply the snow/mud presentation fade to every wrapped and actor camera.
    pub fn with_snow_mud_fade(mut self, fade: f32) -> Self {
        self.fow_opacity_time_snow_mud_day_night[2] = fade;
        self
    }

    /// Fills every field for one camera at one hour.
    ///
    /// `fog` is whether this frame has a fog-of-war input, which is what
    /// [`fow_fade_factor`] gates on; Scene fills `SnowMudFade` from its local
    /// graphics refresh after constructing each camera's frame constants.
    ///
    /// `FoWTime` follows the frame's animation clock, also used by water and
    /// weather particles. Its engine epoch is unrecorded: the window animates
    /// from its own epoch and a dump samples zero (`map::tables` states this).
    /// vFoWOpacity has no active shader read.
    ///
    /// `ShadowFadeFactor` is the camera's, not a constant; see
    /// [`shadow_fade_factor`].
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
            // Sky uses the original camera.
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
            // HSV-authored light defines become linear RGB. Mesh lighting copies these colours with
            // SUN_INTENSITY and MOON_INTENSITY as diffuse alpha; terrain lighting comes from
            // installed shaders.
            ambient_pos_x_shadow_fade: ambient(0, shadow_fade_factor(defines, eye[1])),
            ambient_neg_x_fow_fade: ambient(1, fow_fade_factor(defines, eye[1], fog)),
            // `MinMeshAlpha` and `NegFogMultiplier` are the engine's per-frame
            // 0 and 1 on this path, not defines.
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
