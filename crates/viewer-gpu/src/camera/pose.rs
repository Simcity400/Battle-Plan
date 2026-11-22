
#[cfg(any(test, feature = "test-support"))]
use super::WATER_HEIGHT;
use super::{FAR, FOV_Y, NEAR};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Pose {
    #[default]
    Map,
    Free(Free),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Free {
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub lens: Lens,
}

pub const PITCH_LIMIT: f32 = 1.5690509;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Lens {
    pub const MAP: Self = Self {
        fov_y: FOV_Y,
        near: NEAR,
        far: FAR,
    };

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
    pub fn new(yaw: f32, pitch: f32, lens: Lens) -> Self {
        Self {
            yaw,
            pitch: pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT),
            roll: 0.0,
            lens,
        }
    }

    pub fn looking_at(eye: [f32; 3], target: [f32; 3], lens: Lens) -> Self {
        let [dx, dy, dz] = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
        Self::new(dx.atan2(dz), dy.atan2(dx.hypot(dz)), lens)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn of_map(yaw: f32, height: f32, tilt_lead: f32) -> Self {
        Self::new(yaw, -(height - WATER_HEIGHT).atan2(tilt_lead), Lens::MAP)
    }

    pub fn forward(&self) -> [f32; 3] {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        [sin_yaw * cos_pitch, sin_pitch, cos_yaw * cos_pitch]
    }

    pub fn axes(&self) -> [[f32; 3]; 3] {
        let forward = self.forward();
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let right = [cos_yaw, 0.0, -sin_yaw];
        let up = cross(forward, right);
        let (sin_roll, cos_roll) = self.roll.sin_cos();
        let rolled_right = std::array::from_fn(|axis| right[axis] * cos_roll - up[axis] * sin_roll);
        let rolled_up = std::array::from_fn(|axis| up[axis] * cos_roll + right[axis] * sin_roll);
        [rolled_right, rolled_up, forward]
    }

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
