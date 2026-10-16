//! The game's map camera and projection conventions.
//!
//! The engine keeps a left-handed look-at camera with a fixed horizontal lead:
//! `eye = (cx, h, cz - 30)` looking at `(cx, WATER_HEIGHT, cz)` with up `+Y`.
//! The lead is the user's `camera_tilt * 0.3` (30 at the default 100) and does not depend on the height, so
//! "pitch" is really a constant lead and `height` is the only zoom control.
//!
//! Matrices follow D3D's row-vector convention (`clip = vec4(world, 1) * M`),
//! which is what [`view_projection`](Camera::view_projection) returns as
//! `[row][column]`. WGSL reads a `mat4x4<f32>` uniform as four consecutive
//! column vectors, so writing those rows straight into the buffer gives WGSL
//! the transpose, which is exactly the column-vector matrix `M^T` that yields
//! the same clip position from `M_wgsl * vec4(world, 1)`. Depth lands in 0..1
//! in both D3D and wgpu, so no depth remap is needed.
//!
//! Beside that camera, a [`Pose::Free`] eye looks in any direction with its own lens
//! ([`pose`]); every method below answers for either.

pub mod frustum;
mod pose;

pub use pose::{Free, Lens, PITCH_LIMIT, Pose};

/// Vertical field of view in radians.
pub const FOV_Y: f32 = 0.785;
/// Near plane.
pub const NEAR: f32 = 5.0;
/// Far plane.
pub const FAR: f32 = 4000.0;
/// `WATER_HEIGHT` from `constants.fxh`; the look-at point and water plane.
pub const WATER_HEIGHT: f32 = 9.5;

use std::io;

use super::gradient_math::hsv_to_rgb;

pub mod motion;

/// Projected viewport position and sorting depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapIconProjection {
    pub position: [f32; 2],
    pub depth: f32,
}

impl MapIconProjection {
    /// Where a caller that keeps every anchor places one behind a free eye: outside any viewport,
    /// and deepest.
    pub const BEHIND: Self = Self {
        position: [-1.0e9, -1.0e9],
        depth: f32::MAX,
    };
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Look-at lead north of the eye: the user's camera_tilt multiplied by 0.3.
const TILT_SCALE: f32 = 0.3;
/// The game holds `settings.txt` `camera_tilt` within these as it reads the file.
const TILT_RANGE: [f32; 2] = [5.0, 100.0];

/// `settings.txt` `camera_tilt` as the game holds it after reading the file.
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

/// The `NDefines.NGraphics` operands the camera and the frame constants read,
/// and the user's camera tilt. Every value comes from the recorded content and
/// the configured user folder; nothing falls back to the shipped numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapDefines {
    /// `camera_tilt * 0.3`.
    pub tilt_lead: f32,
    /// `NFrontend.CAMERA_MIN_HEIGHT` and `CAMERA_MAX_HEIGHT`.
    pub heights: [f32; 2],
    /// `CAMERA_OUTSIDE_MAP_DISTANCE_TOP` and `..._BOTTOM`: how far past the map's
    /// top and bottom edges the visible ground may reach.
    pub outside: [f32; 2],
    /// `SUN_HEIGHT`, `SUN_LATITUDE`, `SUN_HEIGHT_WATER`.
    pub sun: [f32; 3],
    /// `MOON_HEIGHT`, `MOON_HEIGHT_WATER`.
    pub moon: [f32; 2],
    /// `SECOND_SUN_LATITUDE`, `SECOND_MOON_LATITUDE`.
    pub second_latitude: [f32; 2],
    /// Political-mode gradient camera-distance and outline-cutoff pairs, using STRATEGIC_REGIONS for
    /// layer one and COUNTRY for layer two.
    pub gradient_borders: [f32; 4],
    /// `DRAW_SHADOWS_CUTOFF` and `DRAW_SHADOWS_FADE_LENGTH`.
    pub shadows: [f32; 2],
    /// `DRAW_FOW_CUTOFF` and `DRAW_FOW_FADE_LENGTH`.
    pub fog: [f32; 2],
    /// The authored `TREE_FADE_NEAR` and `TREE_FADE_FAR` thresholds.
    pub tree_fade: [f32; 2],
    /// `AMBIENT_LIGHT_POS_X`, `_NEG_X`, `_POS_Y`, `_NEG_Y`, `_POS_Z`, `_NEG_Z`,
    /// linear.
    pub ambient: [[f32; 3]; 6],
    /// `SUN_DIFFUSE_COLOR` (linear) and `SUN_INTENSITY`.
    pub sun_diffuse: [f32; 4],
    /// `MOON_DIFFUSE_COLOR` (linear) and `MOON_INTENSITY`.
    pub moon_diffuse: [f32; 4],
    /// `RAID_MAP_ICON_DRAW_DISTANCE_MIN` and `..._MAX`: the camera heights
    /// raid map icons show between in the ordinary map modes.
    pub raid_icons: [f32; 2],
    /// `RAID_MAP_ICON_MAX_DRAW_DISTANCE_IN_RAID_MAP_MODE`: the raid map mode's upper height.
    pub raid_icons_raid_mode: f32,
    /// `SUN_SPECULAR_INTENSITY`.
    pub sun_specular: f32,
    /// `CUBEMAP_INTENSITY`.
    pub cubemap: f32,
    /// `DRAW_COUNTRY_NAMES_CUTOFF`: country names fade in at or above this height.
    pub country_names_cutoff: f32,
    /// The political gradient-border field operands.
    pub gradient_field: super::gradient_math::FieldDefines,
    /// `BORDER_WIDTH`: the full width of a border strip in map pixels.
    pub border_width: f32,
    /// `PROVINCE_BORDER_FADE_NEAR`/`_FAR` and `STATE_BORDER_FADE_NEAR`/`_FAR`.
    pub border_fades: [[f32; 2]; 2],
    /// `CITY_SPRAWL_SHRINK_DISTANCE`, `DRAW_MAP_OBJECTS_CUTOFF` and
    /// `RAILWAY_CAMERA_CUTOFF`: the map-object camera switches.
    pub sprawl_cutoff: f32,
    pub map_objects_cutoff: f32,
    pub railway_cutoff: f32,
    /// `RAILWAY_CAMERA_CUTOFF_SPEED`: the railway opacity's change per second.
    pub railway_cutoff_speed: f32,
}

impl MapDefines {
    /// `camera_tilt` is the configured user folder's `settings.txt` value.
    pub fn load(defines: &installed_content::defines::Defines, camera_tilt: f32) -> io::Result<Self> {
        let define = |section: &str, key: &str| -> io::Result<f32> {
            let value = defines.number(section, key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!("NDefines.{section}.{key} is outside f32")))
            }
        };
        let number = |key: &str| define("NGraphics", key);
        // Convert the authored HSV triple to RGB, then raise each component to power 2.2 for linear colour.
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

    /// The shipped content's values, for synthetic fixtures.
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

/// [`hsv_to_rgb`], then `ToLinear` (`pow 2.2`).
fn hsv_linear(hsv: [f32; 3]) -> [f32; 3] {
    hsv_to_rgb(hsv).map(|channel| channel.powf(2.2))
}

/// Map pixel dimensions come from the province bitmap named by default.map. Shader globals receive
/// them as floats. World units are map pixels; wrapping uses the integer width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapSize {
    pub width: u32,
    pub height: u32,
}

impl MapSize {
    pub fn of(map: &installed_content::map::MapData) -> Self {
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

    /// A vanilla-shaped map for synthetic fixtures.
    #[cfg(any(test, feature = "test-support"))]
    pub const TEST: Self = Self {
        width: 5632,
        height: 2048,
    };
}

/// The map camera: the ground point it looks at, its height and the map it
/// looks at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The look-at point on the water plane, as world `(x, z)`; a free pose's eye.
    pub center: [f32; 2],
    /// Eye height in world units; the only zoom control the engine has.
    pub height: f32,
    pub map: MapSize,
    pub defines: MapDefines,
    /// The game's look-at camera, or a free eye.
    pub pose: Pose,
}

/// A camera given before its map is known, as `--camera` does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub center: [f32; 2],
    pub height: f32,
    pub pose: Pose,
}

impl Placement {
    /// The game's camera looking at `center` from `height`.
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

/// An orthonormal left-handed camera basis: right, up and forward.
struct Basis {
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
}

impl Camera {
    /// The camera without its map.
    pub fn placement(&self) -> Placement {
        Placement {
            center: self.center,
            height: self.height,
            pose: self.pose,
        }
    }

    /// The free pose, `None` for the game's camera.
    pub fn free(&self) -> Option<&Free> {
        match &self.pose {
            Pose::Map => None,
            Pose::Free(free) => Some(free),
        }
    }

    /// The lens the projection uses.
    pub fn lens(&self) -> Lens {
        self.free().map_or(Lens::MAP, |free| free.lens)
    }

    /// The eye position, `vCamPos` in the frame constants.
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

    /// The look-at point: on the water plane for the game's camera, one unit ahead of a free eye.
    pub fn look_at(&self) -> [f32; 3] {
        match &self.pose {
            Pose::Map => [self.center[0], WATER_HEIGHT, self.center[1]],
            Pose::Free(free) => {
                let [eye, forward] = [self.eye(), free.forward()];
                std::array::from_fn(|axis| eye[axis] + forward[axis])
            }
        }
    }

    /// The normalised forward direction, `vCamLookAtDir`.
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
        // LookAtLH: right = normalize(cross(up, forward)), up = cross(forward, right).
        let right = normalize(cross([0.0, 1.0, 0.0], forward));
        Basis {
            up: cross(forward, right),
            right,
            forward,
        }
    }

    /// The view's right column and normalised up column, which world text
    /// quads face along.
    pub fn world_text_axes(&self) -> ([f32; 3], [f32; 3]) {
        let Basis { right, up, .. } = self.basis();
        (right, normalize(up))
    }

    /// The view's right, up and forward unit axes.
    pub fn axes(&self) -> [[f32; 3]; 3] {
        let Basis { right, up, forward } = self.basis();
        [right, up, forward]
    }

    /// `ViewProjectionMatrix` as `[row][column]` in the row-vector convention.
    pub fn view_projection(&self, viewport: [u32; 2]) -> [[f32; 4]; 4] {
        multiply(self.view(), projection(self.lens(), aspect(viewport)))
    }

    pub fn projection(&self, viewport: [u32; 2]) -> [[f32; 4]; 4] {
        projection(self.lens(), aspect(viewport))
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
    // this ( "Map-icon anchoring"); a screen-space pass wants
    // [`project_wrapped`], which is this with the map's x seam accounted for.
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

    /// Map-icon anchors move by whole map widths to within half a width of the camera. Project using
    /// the frame camera, divide by signed w without refusal, and scale coordinates by the integer GUI
    /// extent. The game's camera keeps behind-eye and non-finite results for caller conversion; a
    /// free eye has anchors behind it, and those return `None`.
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

    /// Returns z divided by w without an eye-plane guard.
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

    /// Divides z by signed w, including behind-eye points.
    pub fn map_icon_depth(matrix: &[[f32; 4]; 4], world: [f32; 3]) -> f32 {
        let clip = |column: usize| {
            world[1] * matrix[1][column]
                + world[0] * matrix[0][column]
                + world[2] * matrix[2][column]
                + matrix[3][column]
        };
        clip(2) / clip(3)
    }

    /// Projects a world anchor, trying the map's two x-seam copies as well.
    ///
    /// The map wraps at its width, and the world passes cover a view that
    /// runs past either edge by drawing their geometry again a map width away
    /// (`map::border_pass::wrap_offsets`). A screen-space icon has no geometry
    /// to copy, so it is the anchor that moves instead: a province just across
    /// the seam from the camera is seen a whole map width from where its
    /// `map/unitstacks.txt` position puts it, and projecting that position
    /// unshifted drops the icon off the frame entirely.
    ///
    /// The three candidates are the anchor and the anchor shifted by plus and
    /// minus a map width. **At most one is kept** — see [`wrap_offset`] — so an
    /// anchor never draws twice. `None` only when no candidate projects at all.
    ///
    /// [`wrap_offset`]: Camera::wrap_offset
    pub fn project_wrapped(&self, world: [f32; 3], viewport: [u32; 2]) -> Option<[f32; 2]> {
        let offset = self.wrap_offset(world, viewport);
        self.project([world[0] + offset, world[1], world[2]], viewport)
    }

    /// Which of the map's three x-seam copies of `world` the frame sees, as a
    /// world x translation of `0` or minus or plus the map width.
    ///
    /// The copy kept is the one whose projection lands nearest the middle of
    /// the viewport, which is the one the visible rectangle reaches: the other
    /// two are a whole map width away in world x and so at least the width of
    /// the frame away on screen at every height the engine allows. `0` when no
    /// candidate projects at all, where the caller's own projection fails too.
    ///
    /// A caller that projects two anchors and reads the direction between them
    /// needs both on the *same* copy, which is what this exposes.
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

    /// Where a screen pixel's view ray meets the water plane, as world `(x, z)`.
    ///
    /// `None` when the ray points at or above the horizon. At every height the
    /// engine allows this cannot happen for the game's camera: the steepest lead is
    /// `atan2(30, 50 - 9.5) = 36.5` degrees above the forward axis while half
    /// the vertical field of view is only 22.5 degrees. A free eye looks above it.
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
    pub fn ray(&self, screen: [f32; 2], viewport: [u32; 2]) -> [f32; 3] {
        let width = viewport[0].max(1) as f32;
        let height = viewport[1].max(1) as f32;
        let ndc_x = screen[0] / width * 2.0 - 1.0;
        let ndc_y = 1.0 - screen[1] / height * 2.0;
        let y_scale = 1.0 / (self.lens().fov_y * 0.5).tan();
        // View-space direction that projects onto this NDC point, with z = 1.
        let view = [ndc_x * aspect(viewport) / y_scale, ndc_y / y_scale, 1.0];
        let Basis { right, up, forward } = self.basis();
        [
            right[0] * view[0] + up[0] * view[1] + forward[0] * view[2],
            right[1] * view[0] + up[1] * view[1] + forward[1] * view[2],
            right[2] * view[0] + up[2] * view[1] + forward[2] * view[2],
        ]
    }

    /// integer screen point, inverse projection,
    /// camera-to-world, then subtract the eye and normalise.
    pub fn pick_ray(&self, screen: [f32; 2], viewport: [u32; 2]) -> [f32; 3] {
        let [x, y] = screen.map(|value| value as i32 as f32);
        let x = (x + x) / viewport[0].max(1) as f32 - 1.0;
        let y = 1.0 - (y + y) / viewport[1].max(1) as f32;
        self.ray_at_ndc([x, y], viewport)
    }

    /// The direction the map pans towards a framebuffer pixel: the [`Self::pick_ray`] through it,
    /// kept to its part in the camera's right and up plane, back in world axes, with its height
    /// dropped. It is not normalised, so its length follows the view's field and aspect.
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

    /// Floating NDC coordinates are accepted directly. Shadow footprints use exact corners and centre
    /// without screen rounding.
    pub fn ray_at_ndc(&self, [x, y]: [f32; 2], viewport: [u32; 2]) -> [f32; 3] {
        // Build these inverse terms directly from tan(fov/2),
        // rather than inverting the rounded forward projection.
        let lens = self.lens();
        let tangent = (lens.fov_y * 0.5).tan();
        let reciprocal_w = 1.0 / (1.0 / lens.near);
        let view = [
            x * (tangent * aspect(viewport)) * reciprocal_w,
            y * tangent * reciprocal_w,
            reciprocal_w,
        ];
        let Basis { right, up, forward } = self.basis();
        let up = normalize(up); // Normalize the camera-to-world up axis.
        if self.free().is_some() {
            // A free eye's near plane can be a small fraction of its coordinates, where the
            // game's round trip through the eye's position would lose the ray's direction.
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

    /// Pulls the centre back until the visible ground stays inside the map
    /// grown by `CAMERA_OUTSIDE_MAP_DISTANCE_TOP`/`_BOTTOM`.
    ///
    /// Compare visible z extent with map height plus the top/bottom allowance. While it fits, shift
    /// the camera by edge overshoot; otherwise centre the overshoot. Horizontal edges remain
    /// unclamped because the map wraps.
    ///
    /// The top and bottom allowances constrain the visible ground footprint at every zoom level.
    ///
    /// `--dump --camera` sets the camera directly and never calls this, so a
    /// headless frame still renders exactly the camera it is given. A free eye
    /// keeps its own limits ([`Camera::hold_free`]).
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
        self.clamp_center(viewport);
    }

    /// Centres the map and picks the lowest height that shows all of it.
    ///
    /// The visible ground quad is not a linear function of the height, because
    /// the 30-unit lead is fixed while the eye rises, so this bisects instead of
    /// solving. The engine's `CAMERA_MAX_HEIGHT` of 3000 does not reach the full
    /// width of the shipped map at 16:9, so the result saturates there.
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

    /// The `(x0, z0, x1, z1)` bounding box of the visible water plane, grown by
    /// `margin` on every side so terrain standing above the plane stays covered.
    ///
    /// A free eye's box is what its view volume reaches of the band the map's objects stand in,
    /// out to its [object range](Camera::object_range). Looking wholly above the band it is the
    /// eye's own point, so nothing beyond the margin around it passes.
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

    /// How far from a free eye the map's objects stand: the content's `DRAW_MAP_OBJECTS_CUTOFF`,
    /// the distance the game removes map objects at, or the far plane if nearer. The game's camera
    /// has no such range; its height gates its objects.
    pub fn object_range(&self) -> f32 {
        self.defines.map_objects_cutoff.min(self.lens().far)
    }

    /// How far a map icon at `anchor` stands from the camera, as the content's map-icon distance
    /// cutoffs and grouping distances read it: the game's camera height, or a free eye's distance
    /// to the anchor's nearest map copy.
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

    /// The `(x0, z0, x1, z1)` box of the part of the view volume within view depth `range` and
    /// between heights `band`; `None` when the view misses the band.
    pub fn reach(&self, viewport: [u32; 2], range: f32, band: [f32; 2]) -> Option<[f32; 4]> {
        frustum::bounds(&self.reach_points(viewport, range, band))
    }

    /// The corners of the view volume within view depth `range` and between heights `band`.
    pub fn reach_points(&self, viewport: [u32; 2], range: f32, band: [f32; 2]) -> Vec<[f32; 3]> {
        let near = self.lens().near;
        let corners = frustum::corners(self, viewport, near, range.max(near));
        frustum::in_band(&corners, band[0], band[1])
    }

    /// The view volume's six planes, inside non-negative: left, right, top, bottom, near, far.
    pub fn frustum_planes(&self, viewport: [u32; 2]) -> [[f32; 4]; 6] {
        frustum::planes(&self.view_projection(viewport))
    }

    /// Keeps a free eye on the map: its `x` on the first map copy, its `z` within the map rows and
    /// its height at least `clearance` above `floor` under it and at most `CAMERA_MAX_HEIGHT`
    /// above the water. The game's camera is left alone.
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

    /// Refuses a camera that would produce a degenerate matrix.
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

/// Left-handed perspective projection with normalized depth in 0..1.
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
mod tests;
