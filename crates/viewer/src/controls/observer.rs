//! The free eye the flying and walking modes move: it looks with the mouse or the right stick and
//! moves relative to its heading.
//!
//! | Control | Keyboard and mouse | Controller |
//! |---|---|---|
//! | Look | Mouse, while the map holds the pointer | Right stick |
//! | Move | W A S D | Left stick |
//! | Rise / sink while flying | Space / Shift | Right / left trigger |
//! | Faster | Ctrl | Left stick pressed in |
//! | Walking: start or stop flying | Space twice | South button (A, cross) twice |
//!
//! Flying, the eye moves at a pace that grows with its height above the ground. Walking, it stands
//! [`EYE_HEIGHT`] above the drawn ground or water, on seas and lakes too, and moves at
//! [`WALK_SPEED`]; flying from a walk keeps that pace, and stopping falls back to the ground.
//!
//! The eye never goes below the drawn ground or water ([`Camera::hold_free`]); its near plane
//! shrinks with its height above it so the ground in front of it stays drawn.

use super::{Frame, approach};
use dashboard::protocol::GamepadButton;
use eframe::egui::Key;
use viewer_foundation::map::camera::{Camera, FOV_Y, Free, Lens, PITCH_LIMIT, Pose, WATER_HEIGHT};

/// Radians the view turns per pixel of pointer motion.
pub const MOUSE_LOOK: f32 = 0.0022;
/// Radians per second a full right-stick deflection turns the view across and up or down.
pub const STICK_LOOK: [f32; 2] = [2.2, 1.5];
/// Flying speed in world units per second for each unit of height above the ground, and its
/// limits.
pub const SPEED_PER_HEIGHT: f32 = 1.2;
pub const SLOWEST: f32 = 3.0;
pub const FASTEST: f32 = 900.0;
/// The faster control multiplies the speed by this.
pub const BOOST: f32 = 4.0;
/// Seconds movement and stick looking take to ease in and out.
pub const EASE: f32 = 0.18;
/// The far plane: far enough that the ground runs to the horizon.
pub const FAR: f32 = 20_000.0;
/// The near plane, as a share of the height above the ground, and its limits.
pub const NEAR_PER_HEIGHT: f32 = 0.05;
pub const NEAREST: f32 = 0.05;
pub const FARTHEST_NEAR: f32 = 2.0;
/// The eye keeps at least this many near planes above the ground.
pub const CLEARANCE: f32 = 3.0;
/// A walking eye's height above the drawn ground or water, in world units.
pub const EYE_HEIGHT: f32 = 1.0;
/// Walking speed in world units per second, and the speed of flying from a walk.
pub const WALK_SPEED: f32 = 8.0;
/// World units per second a walker falls back to the ground at.
pub const FALL_SPEED: f32 = 30.0;

/// How the eye moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gait {
    /// Free flight at a pace set by the height above the ground.
    Fly,
    /// On the ground, or flying at walking pace until it lands again.
    Walk { flying: bool },
}

/// A free eye.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observer {
    pub eye: [f32; 3],
    /// Radians clockwise from north.
    pub yaw: f32,
    /// Radians above the horizontal.
    pub pitch: f32,
    gait: Gait,
    /// Eased velocity in world units per second.
    velocity: [f32; 3],
    /// Eased stick turn rates.
    turn: [f32; 2],
}

/// The height of the drawn ground or water under a world `(x, z)`.
pub type Floor<'a> = &'a dyn Fn(f32, f32) -> f32;

impl Observer {
    /// A flying eye where `camera`'s is, looking where it looks.
    pub fn flying(camera: &Camera) -> Self {
        let forward = camera.look_direction();
        Self {
            eye: camera.eye(),
            yaw: forward[0].atan2(forward[2]),
            pitch: forward[1].asin(),
            gait: Gait::Fly,
            velocity: [0.0; 3],
            turn: [0.0; 2],
        }
    }

    /// A walker standing on the ground at world `(x, z)`, facing `yaw` and looking level.
    pub fn walking([x, z]: [f32; 2], yaw: f32, floor: Floor<'_>) -> Self {
        Self {
            eye: [x, ground(floor, x, z) + EYE_HEIGHT, z],
            yaw,
            pitch: 0.0,
            gait: Gait::Walk { flying: false },
            velocity: [0.0; 3],
            turn: [0.0; 2],
        }
    }

    /// The camera this eye sees through, on `base`'s map.
    pub fn camera(&self, base: Camera, floor: Floor<'_>) -> Camera {
        let above = self.eye[1] - ground(floor, self.eye[0], self.eye[2]);
        let near = (above * NEAR_PER_HEIGHT).clamp(NEAREST, FARTHEST_NEAR);
        Camera {
            center: [self.eye[0], self.eye[2]],
            height: self.eye[1],
            pose: Pose::Free(Free::new(
                self.yaw,
                self.pitch,
                Lens {
                    fov_y: FOV_Y,
                    near,
                    far: FAR,
                },
            )),
            ..base
        }
    }

    /// Advances one frame, keeping the eye on the map and above the ground.
    pub fn update(&mut self, frame: &Frame, base: Camera, floor: Floor<'_>) {
        let dt = frame.dt;
        let pad = &frame.pad;
        if let Gait::Walk { flying } = &mut self.gait
            && (frame.double_tapped(Key::Space) || pad.double_tapped(GamepadButton::South))
        {
            *flying = !*flying;
        }
        // Looking: pointer motion is already a distance; the stick is a rate.
        self.turn = [0, 1].map(|axis| {
            let rate = approach(
                self.turn[axis],
                pad.right[axis] * STICK_LOOK[axis],
                EASE,
                dt,
            );
            if rate.abs() < 1e-4 { 0.0 } else { rate }
        });
        self.yaw += frame.look[0] * MOUSE_LOOK + self.turn[0] * dt;
        self.yaw = (self.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        self.pitch = (self.pitch - frame.look[1] * MOUSE_LOOK + self.turn[1] * dt)
            .clamp(-PITCH_LIMIT, PITCH_LIMIT);
        // Moving: relative to the heading, level, with rising and sinking apart.
        let across = (frame.axis(Key::A, Key::D) + pad.left[0]).clamp(-1.0, 1.0);
        let ahead = (frame.axis(Key::S, Key::W) + pad.left[1]).clamp(-1.0, 1.0);
        let up = match self.gait {
            Gait::Fly | Gait::Walk { flying: true } => (f32::from(
                i8::from(frame.held(Key::Space)) - i8::from(frame.modifiers.shift),
            ) + pad.triggers[1]
                - pad.triggers[0])
                .clamp(-1.0, 1.0),
            Gait::Walk { flying: false } => 0.0,
        };
        let below = ground(floor, self.eye[0], self.eye[2]);
        let mut speed = match self.gait {
            Gait::Fly => ((self.eye[1] - below) * SPEED_PER_HEIGHT).clamp(SLOWEST, FASTEST),
            Gait::Walk { .. } => WALK_SPEED,
        };
        if frame.modifiers.ctrl || pad.held(GamepadButton::LeftStick) {
            speed *= BOOST;
        }
        let (sin, cos) = self.yaw.sin_cos();
        let target = [
            (across * cos + ahead * sin) * speed,
            up * speed,
            (-across * sin + ahead * cos) * speed,
        ];
        self.velocity = [0, 1, 2].map(|axis| {
            let value = approach(self.velocity[axis], target[axis], EASE, dt);
            if value.abs() < 1e-3 { 0.0 } else { value }
        });
        for axis in 0..3 {
            self.eye[axis] += self.velocity[axis] * dt;
        }
        if let Gait::Walk { flying } = self.gait {
            let stand = ground(floor, self.eye[0], self.eye[2]) + EYE_HEIGHT;
            if !flying {
                // Fall back to the ground, and follow it up and down.
                self.eye[1] = (self.eye[1] - FALL_SPEED * dt).max(stand);
            }
            self.eye[1] = self.eye[1].max(stand);
        }
        let mut camera = self.camera(base, floor);
        let clearance = camera.lens().near * CLEARANCE;
        camera.hold_free(floor, clearance);
        self.eye = camera.eye();
    }
}

/// The drawn ground or water under a world `(x, z)`.
fn ground(floor: Floor<'_>, x: f32, z: f32) -> f32 {
    floor(x, z).max(WATER_HEIGHT)
}

#[cfg(test)]
mod tests;
