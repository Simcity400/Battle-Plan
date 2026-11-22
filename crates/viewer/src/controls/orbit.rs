
use super::{Frame, approach};
use dashboard_core::protocol::GamepadButton;
use std::f32::consts::{PI, TAU};
use viewer_gpu::camera::{Camera, FOV_Y, Free, Lens, NEAR, Pose, WATER_HEIGHT};

pub const LENS: Lens = Lens {
    fov_y: FOV_Y,
    near: NEAR,
    far: 20_000.0,
};
pub const LOWEST_ELEVATION: f32 = 12.0 * PI / 180.0;
pub const HIGHEST_ELEVATION: f32 = 89.0 * PI / 180.0;
pub const PAN_SPEED: f32 = 0.9;
pub const PAN_BOOST: f32 = 2.5;
pub const TURN_SPEED: f32 = 1.6;
pub const TILT_SPEED: f32 = 0.9;
pub const ZOOM_SPEED: f32 = 1.4;
pub const WHEEL_ZOOM: f32 = 0.002;
pub const EASE: f32 = 0.12;
pub const RESET_EASE: f32 = 0.15;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Orbit {
    yaw: f32,
    tilt: f32,
    pan: [f32; 2],
    turn: f32,
    lean: f32,
    zoom: f32,
    resetting: bool,
}

impl Orbit {
    pub fn is_native(&self) -> bool {
        self.yaw == 0.0 && self.tilt == 0.0
    }

    pub fn moving(&self) -> bool {
        self.resetting
            || self.pan != [0.0; 2]
            || self.turn != 0.0
            || self.lean != 0.0
            || self.zoom != 0.0
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn view(&self, map: Camera) -> Camera {
        if self.is_native() {
            return map;
        }
        let rise = map.height - WATER_HEIGHT;
        let native = rise.atan2(map.defines.tilt_lead);
        let elevation = (native - self.tilt).clamp(LOWEST_ELEVATION, HIGHEST_ELEVATION);
        let lead = rise / elevation.tan();
        let (sin, cos) = self.yaw.sin_cos();
        Camera {
            center: [map.center[0] - sin * lead, map.center[1] - cos * lead],
            pose: Pose::Free(Free::new(self.yaw, -elevation, LENS)),
            ..map
        }
    }

    fn tilt_range(map: &Camera) -> [f32; 2] {
        let native = (map.height - WATER_HEIGHT).atan2(map.defines.tilt_lead);
        [
            (native - HIGHEST_ELEVATION).min(0.0),
            (native - LOWEST_ELEVATION).max(0.0),
        ]
    }

    pub fn update(&mut self, frame: &Frame, map: &mut Camera, keys: [f32; 2]) -> bool {
        let pad = &frame.pad;
        let dt = frame.dt;
        if !pad.active() && !self.moving() && keys == [0.0; 2] {
            return false;
        }
        if pad.pressed(GamepadButton::RightStick) {
            self.resetting = true;
        }
        if pad.right != [0.0; 2] {
            self.resetting = false;
        }
        let boost = if pad.held(GamepadButton::LeftStick) {
            PAN_BOOST
        } else {
            1.0
        };
        let push = [pad.left[0] + keys[0], pad.left[1] + keys[1]];
        let length = push[0].hypot(push[1]).max(1.0);
        let target = [
            push[0] / length * PAN_SPEED * boost,
            push[1] / length * PAN_SPEED * boost,
        ];
        self.pan = target_eased(self.pan, target, dt);
        if self.resetting {
            self.turn = 0.0;
            self.lean = 0.0;
        } else {
            self.turn = settle(approach(self.turn, pad.right[0] * TURN_SPEED, EASE, dt));
            self.lean = settle(approach(self.lean, pad.right[1] * TILT_SPEED, EASE, dt));
        }
        let pull = pad.triggers[1] - pad.triggers[0];
        self.zoom = settle(approach(self.zoom, pull * ZOOM_SPEED, EASE, dt));
        let before = (*self, map.placement());
        let (sin, cos) = self.yaw.sin_cos();
        let scale = (map.height - WATER_HEIGHT).max(1.0) * dt;
        map.center[0] += (self.pan[0] * cos + self.pan[1] * sin) * scale;
        map.center[1] += (-self.pan[0] * sin + self.pan[1] * cos) * scale;
        let [low, high] = map.defines.heights;
        map.height = (map.height * (-self.zoom * dt).exp()).clamp(low, high);
        if self.resetting {
            let yaw = (self.yaw + PI).rem_euclid(TAU) - PI;
            self.yaw = approach(yaw, 0.0, RESET_EASE, dt);
            self.tilt = approach(self.tilt, 0.0, RESET_EASE, dt);
            if self.yaw.abs() < 1e-3 && self.tilt.abs() < 1e-3 {
                self.yaw = 0.0;
                self.tilt = 0.0;
                self.resetting = false;
            }
        } else {
            self.yaw = (self.yaw + self.turn * dt + PI).rem_euclid(TAU) - PI;
            let [lowest, highest] = Self::tilt_range(map);
            self.tilt = (self.tilt + self.lean * dt).clamp(lowest, highest);
            if self.yaw.abs() < f32::EPSILON && self.tilt.abs() < f32::EPSILON {
                self.yaw = 0.0;
                self.tilt = 0.0;
            }
        }
        hold(map);
        (*self, map.placement()) != before
    }

    pub fn pan(&self, map: &mut Camera, from: [f32; 2], to: [f32; 2], viewport: [u32; 2]) {
        let view = self.view(*map);
        let (Some(from), Some(to)) = (
            view.unproject_ground(from, viewport),
            view.unproject_ground(to, viewport),
        ) else {
            return;
        };
        map.center[0] += from[0] - to[0];
        map.center[1] += from[1] - to[1];
        hold(map);
    }

    pub fn zoom(&self, map: &mut Camera, factor: f32, screen: [f32; 2], viewport: [u32; 2]) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let Some(before) = self.view(*map).unproject_ground(screen, viewport) else {
            return;
        };
        let [low, high] = map.defines.heights;
        map.height = (map.height / factor).clamp(low, high);
        let Some(after) = self.view(*map).unproject_ground(screen, viewport) else {
            return;
        };
        map.center[0] += before[0] - after[0];
        map.center[1] += before[1] - after[1];
        hold(map);
    }
}

fn target_eased(value: [f32; 2], target: [f32; 2], dt: f32) -> [f32; 2] {
    [0, 1].map(|axis| settle(approach(value[axis], target[axis], EASE, dt)))
}

fn settle(rate: f32) -> f32 {
    if rate.abs() < 1e-4 { 0.0 } else { rate }
}

fn hold(map: &mut Camera) {
    map.center[1] = map.center[1].clamp(0.0, map.map.y());
}

#[cfg(test)]
mod tests;
