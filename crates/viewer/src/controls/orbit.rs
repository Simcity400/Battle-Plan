//! A controller's orbit around the game's map camera.
//!
//! The map camera looks at a ground point from a height, its eye the user's tilt lead south of
//! it. The orbit keeps that point and height and adds a heading and a tilt: the eye circles the
//! point and leans from the game's own pitch towards the horizon or straight down. With neither,
//! the camera is exactly the game's, so a map nobody orbits draws as it always has.
//!
//! | Control | Action |
//! |---|---|
//! | Left stick | Move the map, relative to the heading; faster while the left stick is pressed in |
//! | Right stick left/right | Turn the view around the point it looks at |
//! | Right stick up/down | Tilt towards the horizon / towards straight down |
//! | Right trigger / left trigger | Zoom in / out |
//! | Right stick press | Ease back to the game's north-up view |
//!
//! Moves ease in and out ([`super::approach`]), scale with the height so the map crosses the
//! screen at the same pace at every zoom, and use wall-clock time. While the view is turned or
//! tilted, the arrow keys and edge scrolling move the map through the orbit as the left stick
//! does, the middle-button drag keeps the ground point under the pointer and the wheel zooms
//! towards the point under the pointer; with neither, they are the game's own camera law.

use super::{Frame, approach};
use dashboard::protocol::GamepadButton;
use std::f32::consts::{PI, TAU};
use viewer_foundation::map::camera::{Camera, FOV_Y, Free, Lens, NEAR, Pose, WATER_HEIGHT};

/// The orbiting camera's lens: the game's field of view and near plane, and a far plane that
/// reaches the horizon when tilted.
pub const LENS: Lens = Lens {
    fov_y: FOV_Y,
    near: NEAR,
    far: 20_000.0,
};
/// The shallowest view, in radians below the horizontal: the top of the frame then shows sky.
pub const LOWEST_ELEVATION: f32 = 12.0 * PI / 180.0;
/// The steepest view: just short of straight down.
pub const HIGHEST_ELEVATION: f32 = 89.0 * PI / 180.0;
/// Camera heights per second a full left-stick deflection moves the map.
pub const PAN_SPEED: f32 = 0.9;
/// The left-stick press multiplies the move by this.
pub const PAN_BOOST: f32 = 2.5;
/// Radians per second a full right-stick deflection turns.
pub const TURN_SPEED: f32 = 1.6;
/// Radians per second a full right-stick deflection tilts.
pub const TILT_SPEED: f32 = 0.9;
/// The height's logarithm changes by this per second at a full trigger pull.
pub const ZOOM_SPEED: f32 = 1.4;
/// The height's logarithm changes by this per point of the window's smoothed wheel scroll.
pub const WHEEL_ZOOM: f32 = 0.002;
/// Seconds the moves take to ease in and out.
pub const EASE: f32 = 0.12;
/// Seconds the return to the game's view takes to settle.
pub const RESET_EASE: f32 = 0.15;

/// The heading and tilt a controller adds to the map camera.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Orbit {
    /// Radians clockwise from north.
    yaw: f32,
    /// Radians the view leans from the game's pitch towards the horizon (negative: towards
    /// straight down).
    tilt: f32,
    /// Eased rates: pan `x`/`z` in heights per second, turn, tilt and zoom.
    pan: [f32; 2],
    turn: f32,
    lean: f32,
    zoom: f32,
    resetting: bool,
}

impl Orbit {
    /// Whether the camera is the game's own.
    pub fn is_native(&self) -> bool {
        self.yaw == 0.0 && self.tilt == 0.0
    }

    /// Whether the orbit is still easing.
    pub fn moving(&self) -> bool {
        self.resetting
            || self.pan != [0.0; 2]
            || self.turn != 0.0
            || self.lean != 0.0
            || self.zoom != 0.0
    }

    /// Back to the game's camera at once.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The camera this orbit makes of the game's camera at `map`'s point and height.
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

    /// The range of tilts the camera at `height` takes; the game's own pitch is always in it,
    /// even high up where it is steeper than [`HIGHEST_ELEVATION`].
    fn tilt_range(map: &Camera) -> [f32; 2] {
        let native = (map.height - WATER_HEIGHT).atan2(map.defines.tilt_lead);
        [
            (native - HIGHEST_ELEVATION).min(0.0),
            (native - LOWEST_ELEVATION).max(0.0),
        ]
    }

    /// Advances one frame of controller input on the game camera `map`, keeping its height
    /// within the content's limits and the point it looks at on the map. `keys` adds a move from
    /// the keyboard or the pointer, `x` right and `y` up on the screen, as the left stick gives
    /// one. Returns whether anything moved, so the caller keeps drawing frames; without input or
    /// easing it touches nothing.
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
            // The return owns the heading and tilt: no turn or lean still easing out carries
            // past it, so it ends exactly on the game's camera.
            self.turn = 0.0;
            self.lean = 0.0;
        } else {
            self.turn = settle(approach(self.turn, pad.right[0] * TURN_SPEED, EASE, dt));
            self.lean = settle(approach(self.lean, pad.right[1] * TILT_SPEED, EASE, dt));
        }
        let pull = pad.triggers[1] - pad.triggers[0];
        self.zoom = settle(approach(self.zoom, pull * ZOOM_SPEED, EASE, dt));
        let before = (*self, map.placement());
        // Pan along the view's own right and forward on the ground.
        let (sin, cos) = self.yaw.sin_cos();
        let scale = (map.height - WATER_HEIGHT).max(1.0) * dt;
        map.center[0] += (self.pan[0] * cos + self.pan[1] * sin) * scale;
        map.center[1] += (-self.pan[0] * sin + self.pan[1] * cos) * scale;
        let [low, high] = map.defines.heights;
        map.height = (map.height * (-self.zoom * dt).exp()).clamp(low, high);
        if self.resetting {
            // The shorter way round, then settle exactly on the game's view.
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

    /// Drags the ground point under `from` to `to`, as the map's drag does, through the orbiting
    /// view: the game camera's point moves by the ground distance between them.
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

    /// Divides the height by `factor`, keeping the ground point under `screen` there through the
    /// orbiting view.
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

/// Eases a two-axis rate towards its target.
fn target_eased(value: [f32; 2], target: [f32; 2], dt: f32) -> [f32; 2] {
    [0, 1].map(|axis| settle(approach(value[axis], target[axis], EASE, dt)))
}

/// A rate this small has stopped.
fn settle(rate: f32) -> f32 {
    if rate.abs() < 1e-4 { 0.0 } else { rate }
}

/// Keeps the point the camera looks at within the map rows, as the game holds its own look-at
/// point; `x` runs on across the seam as the game camera's does.
fn hold(map: &mut Camera) {
    map.center[1] = map.center[1].clamp(0.0, map.map.y());
}

#[cfg(test)]
mod tests;
