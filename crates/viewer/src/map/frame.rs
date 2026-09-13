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
//! Values come from research R1 "Constant buffer 0 at the reference frame".

use bytemuck::{Pod, Zeroable};

use super::camera::{Camera, MAP_SIZE_X, MAP_SIZE_Y};

/// `SUN_HEIGHT` (`00_graphics.lua:1043-1050`).
pub const SUN_HEIGHT: f32 = 600.0;
/// `SUN_LATITUDE`.
pub const SUN_LATITUDE: f32 = 848.0;
/// `SUN_HEIGHT_WATER`, carried in `vVirtualSunPos.w` for `CalculateSunDirectionWater`.
pub const SUN_HEIGHT_WATER: f32 = 5000.0;
/// `MOON_HEIGHT`.
pub const MOON_HEIGHT: f32 = 600.0;
/// `MOON_HEIGHT_WATER`, carried in `vVirtualMoonPos.w`.
pub const MOON_HEIGHT_WATER: f32 = 550.0;
/// `SECOND_SUN_LATITUDE` and `SECOND_MOON_LATITUDE`, both 100.
pub const SECOND_LATITUDE: f32 = 100.0;

/// Solar declination in degrees, measured from the terminator in
/// `array_loaded.png` on 23 August 1943 (R1 section 2.4, rms 0.94 px).
///
/// UNVERIFIED-seasonal: one date gives one sample, so the amplitude and phase
/// of the engine's seasonal law are unknown. Every hour the viewer draws uses
/// this constant, which is correct only near late August.
pub const SUN_DECLINATION_DEGREES: f32 = 1.62;

/// `GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_COUNTRY` (`00_graphics.lua:890`).
///
/// The `GRADIENT_BORDERS_*_<NAME>` suffixes are **map modes**, not channels:
/// `00_graphics.lua:890-902` lists COUNTRY, STATE, SUPPLY_AREA,
/// STRATEGIC_REGIONS, RESISTANCE, FACTIONS, TERRAIN, INTEL_LEDGER, DIPLOMACY,
/// DIPLOMACY_ON_INTEL_LEDGER and PEACE_CONFERENCE — eleven of them for a vector
/// with two slots, and TERRAIN and FACTIONS are map modes with no plausible
/// channel meaning. R2 section 1.3 read them as one per channel and R1
/// section 7 carried that reading forward; both channels of the political
/// (country) map mode take this one value.
pub const GB_CAM_DIST_OVERRIDE: f32 = 0.0;
/// `GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_STATE` (`00_graphics.lua:891`),
/// which belongs to the *state* map mode the viewer does not draw yet.
#[allow(dead_code)]
pub const GB_CAM_DIST_OVERRIDE_STATE: f32 = 0.4;
/// `GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY` (`00_graphics.lua:880`), on both
/// channels for the same reason as the camera override.
pub const GB_OUTLINE_CUTOFF: f32 = 0.973;

/// `GB_TextureHeight`: the gradient-border texture height in texels. The two
/// channels are stacked vertically, so this is twice a map half-height plus the
/// two half-texel guards (`standardfuncsgfx.fxh:903`).
pub const GB_TEXTURE_HEIGHT: f32 = 2050.0;

/// Every value the frame constants need that is not the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayNight {
    /// `DayNight_Hour_SunDir.x`: `(hour mod 24) / 24` on the raw replay hour.
    pub hour_fraction: f32,
    /// Solar declination in radians.
    pub declination: f32,
    /// `vFoWOpacity_..._MaxGameSpeed.w`: the day/night strength, 1 while the
    /// game is not at maximum speed (R1 section 2.5).
    pub strength: f32,
}

impl DayNight {
    /// The day/night state at a replay hour.
    ///
    /// R1 section 2.2: the engine's `(hour - 43800000) mod 24` is the raw hour
    /// modulo 24 because `43800000 mod 24 == 0`, and it is one less than the
    /// hour the interface shows. That offset is the engine's, not a defect.
    pub fn at_hour(hour: i32) -> Self {
        Self {
            hour_fraction: hour_fraction(hour),
            declination: SUN_DECLINATION_DEGREES.to_radians(),
            strength: 1.0,
        }
    }

    /// `DayNight_Hour_SunDir`: the hour fraction and the night direction.
    ///
    /// The direction's `x` is zero because `CalcGlobeNormal` already bakes the
    /// hour rotation into the globe normal (R1 section 2.4).
    pub fn hour_sun_dir(&self) -> [f32; 4] {
        [
            self.hour_fraction,
            0.0,
            -self.declination.sin(),
            self.declination.cos(),
        ]
    }

    /// `vVirtualSunPos.xyz` (R1 section 2.3).
    pub fn sun_position(&self) -> [f32; 3] {
        [
            (1.0 - self.hour_fraction) * MAP_SIZE_X,
            SUN_HEIGHT,
            SUN_LATITUDE,
        ]
    }

    /// `vVirtualMoonPos.xyz`.
    ///
    /// UNVERIFIED (R1 section 2.3): the decompile computes the moon's `z` as
    /// `MAP_SIZE_Y - sunPos.y`, that is `2048 - 600`, rather than the more
    /// plausible `MAP_SIZE_Y - SUN_LATITUDE`. The decompiled form is used here.
    pub fn moon_position(&self) -> [f32; 3] {
        [
            wrap_x(self.sun_position()[0] + MAP_SIZE_X * 0.5),
            MOON_HEIGHT,
            MAP_SIZE_Y - SUN_HEIGHT,
        ]
    }
}

/// `(hour mod 24) / 24`, handling negative hours the way a modulo should.
pub fn hour_fraction(hour: i32) -> f32 {
    (hour.rem_euclid(24)) as f32 / 24.0
}

fn wrap_x(x: f32) -> f32 {
    x - MAP_SIZE_X * (x / MAP_SIZE_X).floor()
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
    /// Fills every field for one camera at one hour.
    ///
    /// `FOWFadeFactor` is 0 and `vFoWOpacity`/`FoWTime` are 0 because the
    /// viewer is omniscient (`PLAN.md` section 2); `SnowMudFade` is 0 because
    /// the replay never interpolates between hours (R1 section 6.1).
    /// `ShadowFadeFactor` is 1: `shadows=no` leaves nothing to cast, and the
    /// terrain's `fShadowTerm = max(..., 0.1)` collapses to 1 (R1 section 3.4).
    pub fn new(camera: Camera, viewport: [u32; 2], day_night: DayNight, global_time: f32) -> Self {
        let sun = day_night.sun_position();
        let moon = day_night.moon_position();
        let eye = camera.eye();
        let look = camera.look_direction();
        let view_projection = camera.view_projection(viewport);
        Self {
            view_projection,
            // A camera that projects at all is invertible; fall back to the
            // identity rather than fail, since no map pass reads this field yet.
            inv_view_projection: camera.inverse(viewport).unwrap_or(IDENTITY),
            gb_cam_dist_override_outline_cutoff: [
                GB_CAM_DIST_OVERRIDE,
                GB_CAM_DIST_OVERRIDE,
                GB_OUTLINE_CUTOFF,
                GB_OUTLINE_CUTOFF,
            ],
            virtual_sun_pos: [sun[0], sun[1], sun[2], SUN_HEIGHT_WATER],
            virtual_moon_pos: [moon[0], moon[1], moon[2], MOON_HEIGHT_WATER],
            second_virtual_sun_pos: [
                wrap_x(sun[0] + MAP_SIZE_X * 0.5),
                SUN_HEIGHT,
                SECOND_LATITUDE,
                SUN_HEIGHT_WATER,
            ],
            second_virtual_moon_pos: [
                wrap_x(moon[0] + MAP_SIZE_X * 0.5),
                MOON_HEIGHT,
                SECOND_LATITUDE,
                MOON_HEIGHT_WATER,
            ],
            cam_pos_hdr_range: [eye[0], eye[1], eye[2], 1.0],
            cam_look_at_dir_global_time: [look[0], look[1], look[2], global_time],
            fow_opacity_time_snow_mud_day_night: [0.0, 0.0, 0.0, day_night.strength],
            day_night_hour_sun_dir: day_night.hour_sun_dir(),
            // The linear RGB of the HSV-authored defines (R1 section 3.2).
            // Only `pdxmesh` reads these; the terrain uses `constants.fxh`.
            ambient_pos_x_shadow_fade: [0.0233, 0.0289, 0.0170, 1.0],
            ambient_neg_x_fow_fade: [0.2591, 0.3251, 0.2848, 0.0],
            ambient_pos_y_min_mesh_alpha: [0.0, 0.0, 0.0, 0.0],
            ambient_neg_y_neg_fog: [0.0, 0.0, 0.0, 0.0],
            ambient_pos_z: [0.5197, 0.5788, 0.8382, 0.0],
            ambient_neg_z_cubemap: [0.6318, 0.7091, 0.7909, 1.0],
            sun_diffuse_intensity: [1.0, 1.0, 1.0, 1.0],
            moon_diffuse_intensity: [0.2176, 0.5566, 1.0, 2.5],
            gb_texture_height_sun_specular: [GB_TEXTURE_HEIGHT, 1.0, 0.0, 0.0],
        }
    }
}

const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE_HOUR: i32 = 60_826_314;

    #[test]
    fn the_reference_hour_reproduces_the_report_sun_and_moon() {
        let day_night = DayNight::at_hour(REFERENCE_HOUR);
        assert_eq!(day_night.hour_fraction, 0.75);
        assert_eq!(day_night.sun_position(), [1408.0, 600.0, 848.0]);
        assert_eq!(day_night.moon_position(), [4224.0, 600.0, 1448.0]);
        let direction = day_night.hour_sun_dir();
        assert_eq!(direction[0], 0.75);
        assert_eq!(direction[1], 0.0);
        assert!((direction[2] - -0.0282).abs() < 1e-4, "{direction:?}");
        assert!((direction[3] - 0.99960).abs() < 1e-4, "{direction:?}");
    }

    #[test]
    fn the_hour_fraction_wraps_the_way_the_engine_does() {
        assert_eq!(hour_fraction(0), 0.0);
        assert_eq!(hour_fraction(43_800_000), 0.0);
        assert_eq!(hour_fraction(REFERENCE_HOUR + 1), 19.0 / 24.0);
        assert_eq!(hour_fraction(60_832_933), 13.0 / 24.0);
        assert_eq!(hour_fraction(-1), 23.0 / 24.0);
    }

    #[test]
    fn the_constant_buffer_matches_the_reference_frame() {
        let camera = super::Camera {
            center: [3010.0, 1568.0],
            height: 310.178,
        };
        let constants =
            FrameConstants::new(camera, [2048, 1152], DayNight::at_hour(REFERENCE_HOUR), 0.0);
        assert_eq!(constants.virtual_sun_pos, [1408.0, 600.0, 848.0, 5000.0]);
        assert_eq!(constants.virtual_moon_pos, [4224.0, 600.0, 1448.0, 550.0]);
        assert_eq!(
            constants.second_virtual_sun_pos,
            [4224.0, 600.0, 100.0, 5000.0]
        );
        assert_eq!(
            constants.second_virtual_moon_pos,
            [1408.0, 600.0, 100.0, 550.0]
        );
        assert_eq!(constants.cam_pos_hdr_range, [3010.0, 310.178, 1538.0, 1.0]);
        let look = constants.cam_look_at_dir_global_time;
        assert!((look[0]).abs() < 1e-6);
        assert!((look[1] - -0.995059).abs() < 1e-5, "{look:?}");
        assert!((look[2] - 0.099282).abs() < 1e-5, "{look:?}");
        assert_eq!(
            constants.fow_opacity_time_snow_mud_day_night,
            [0.0, 0.0, 0.0, 1.0]
        );
        // Both channels take the COUNTRY map mode's defines; `map::political`
        // leaves channel 2 empty, so its override would not reach a pixel
        // either way, but 0.4 is the state map mode's value and not this one's.
        assert_eq!(
            constants.gb_cam_dist_override_outline_cutoff,
            [0.0, 0.0, 0.973, 0.973]
        );
        assert_eq!(constants.gb_texture_height_sun_specular[0], 2050.0);
        assert_eq!(constants.moon_diffuse_intensity, [0.2176, 0.5566, 1.0, 2.5]);
    }

    #[test]
    fn the_buffer_layout_matches_the_hlsl_registers() {
        assert_eq!(std::mem::size_of::<FrameConstants>(), 416);
        let zero = FrameConstants::zeroed();
        let base = std::ptr::from_ref(&zero) as usize;
        let offset = |field: &[f32; 4]| std::ptr::from_ref(field) as usize - base;
        assert_eq!(offset(&zero.gb_cam_dist_override_outline_cutoff), 128);
        assert_eq!(offset(&zero.cam_pos_hdr_range), 208);
        assert_eq!(offset(&zero.day_night_hour_sun_dir), 256);
        assert_eq!(offset(&zero.ambient_pos_z), 336);
        assert_eq!(offset(&zero.gb_texture_height_sun_specular), 400);
    }
}
