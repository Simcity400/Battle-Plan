//! Free camera poses beside the game's look-at camera.
//!
//! [`Pose::Map`] is the game's camera and keeps every result the map has always drawn.
//! [`Pose::Free`] places the eye at the camera's `(center.x, height, center.z)` and turns it by
//! its own heading, pitch and roll, with its own lens, so a view can look at the horizon, fly or
//! walk. World axes are the game's: `x` east, `y` up, `z` north, left-handed.

#[cfg(any(test, feature = "test-support"))]
use super::WATER_HEIGHT;
use super::{FAR, FOV_Y, NEAR};

/// How the camera looks at the map.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Pose {
    /// The game's look-at camera: `center` is the water-plane point it looks at and the eye
    /// stands the user's tilt lead south of it at `height`.
    #[default]
    Map,
    /// An eye at `(center.x, height, center.z)` turned by its own orientation.
    Free(Free),
}

/// A free eye's orientation and lens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Free {
    /// Heading in radians, clockwise from north (`+z`) seen from above.
    pub yaw: f32,
    /// Radians above the horizontal; negative looks down. Held within [`PITCH_LIMIT`].
    pub pitch: f32,
    /// Bank in radians: positive lowers the view's right side.
    pub roll: f32,
    pub lens: Lens,
}

/// The steepest pitch a free pose takes, just short of straight up or down so its heading stays
/// meaningful.
pub const PITCH_LIMIT: f32 = 1.5690509; // 89.9 degrees

/// A perspective lens: vertical field of view and depth range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    /// Vertical field of view in radians.
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Lens {
    /// The game's map lens.
    pub const MAP: Self = Self {
        fov_y: FOV_Y,
        near: NEAR,
        far: FAR,
    };

    /// Whether the lens makes a usable projection.
    pub fn is_valid(&self) -> bool {
        self.fov_y.is_finite()
            && self.fov_y > 0.0
            && self.fov_y < std::f32::consts::PI
            && self.near.is_finite()
            && self.far.is_finite()
            && self.near > 0.0
            && self.far > self.near
    }
}

impl Free {
    /// A level pose looking along `yaw` with the given lens.
    pub fn new(yaw: f32, pitch: f32, lens: Lens) -> Self {
        Self {
            yaw,
            pitch: pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT),
            roll: 0.0,
            lens,
        }
    }

    /// The pose at `eye` looking at `target`, unrolled.
    pub fn looking_at(eye: [f32; 3], target: [f32; 3], lens: Lens) -> Self {
        let [dx, dy, dz] = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
        Self::new(dx.atan2(dz), dy.atan2(dx.hypot(dz)), lens)
    }

    /// The free pose that sees exactly what the game's camera at `height` with `tilt_lead`
    /// sees, turned to face `yaw`; the eye stands the lead behind its look-at point.
    #[cfg(any(test, feature = "test-support"))]
    pub fn of_map(yaw: f32, height: f32, tilt_lead: f32) -> Self {
        Self::new(yaw, -(height - WATER_HEIGHT).atan2(tilt_lead), Lens::MAP)
    }

    /// The unit forward direction.
    pub fn forward(&self) -> [f32; 3] {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        [sin_yaw * cos_pitch, sin_pitch, cos_yaw * cos_pitch]
    }

    /// Right, up and forward unit axes, rolled.
    pub fn axes(&self) -> [[f32; 3]; 3] {
        let forward = self.forward();
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        // The level right axis, then up = forward x right, which is the left-handed look-at's.
        let right = [cos_yaw, 0.0, -sin_yaw];
        let up = cross(forward, right);
        let (sin_roll, cos_roll) = self.roll.sin_cos();
        let rolled_right = std::array::from_fn(|axis| right[axis] * cos_roll - up[axis] * sin_roll);
        let rolled_up = std::array::from_fn(|axis| up[axis] * cos_roll + right[axis] * sin_roll);
        [rolled_right, rolled_up, forward]
    }

    /// Whether the orientation is a number.
    pub fn is_finite(&self) -> bool {
        self.yaw.is_finite() && self.pitch.is_finite() && self.roll.is_finite()
    }
}

pub(super) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
