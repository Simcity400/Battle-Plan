//! One input layer for the keyboard, the mouse and controllers, and the camera controllers that
//! read it.
//!
//! [`Input`] turns each frame's window events and the latest controller state into a [`Frame`]:
//! wall-clock time since the last frame (never the replay's speed), held and newly pressed keys
//! (by their place on the keyboard, as games read movement keys),
//! pointer motion while the map holds the pointer, wheel turns and the controller after its dead
//! zones. Clicking the map in a free mode captures the pointer; Escape and losing keyboard focus
//! release it and forget every held key, so no key keeps moving the camera after the map loses
//! the user's attention.
//!
//! Controllers reach the viewer from the desktop host ([`GamepadMessage`]): only the process
//! owning the foreground window receives their input, and the map's window belongs to the
//! host's. The host alone decides which map holds the controller, by the window in front; the
//! map's own focus never touches it. A map taking it reads what is held without pressing
//! anything; only a release lets go of everything, without pressing anything, and drops states
//! sent before it, as a map drops states of any handoff it does not hold. Presses count against
//! the last state read, so a button held through a handoff or a pause is never pressed twice. Keyboard and mouse input arrives at the map's
//! own window.
//!
//! [`orbit`] moves the game's map camera with a controller; [`observer`] is the free eye the
//! flying and walking modes move.

pub mod observer;
pub mod orbit;

use dashboard::protocol::{GamepadButton, GamepadMessage, GamepadOwner, GamepadState};
use eframe::egui;
use std::collections::BTreeSet;
use std::time::Instant;

/// The longest frame a controller integrates: after a stall the camera moves no further than
/// this many seconds' worth.
pub const LONGEST_STEP: f32 = 0.1;

/// A stick deflection below this fraction of full counts as centred: worn sticks rest off centre.
pub const STICK_DEAD_ZONE: f32 = 0.18;
/// A stick deflection beyond this fraction counts as full: many sticks never reach 1.
pub const STICK_SATURATION: f32 = 0.95;
/// The exponent between deflection past the dead zone and response, for fine control near the
/// centre: half deflection gives about a third of full speed.
pub const STICK_CURVE: f32 = 1.6;
/// A trigger pulled less than this counts as released.
pub const TRIGGER_DEAD_ZONE: f32 = 0.06;
/// Two presses of a key within this many seconds are a double tap.
pub const DOUBLE_TAP: f32 = 0.3;

/// A controller after its dead zones and response curves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    /// Left stick, `x` right and `y` up, within the unit circle.
    pub left: [f32; 2],
    /// Right stick, the same way.
    pub right: [f32; 2],
    /// Left then right trigger, 0..=1.
    pub triggers: [f32; 2],
    /// Held buttons, [`GamepadButton`] bits.
    pub held: u32,
    /// Buttons pressed since the last frame.
    pub pressed: u32,
    /// Buttons whose press this frame followed another within [`DOUBLE_TAP`].
    pub double_tapped: u32,
}

impl Pad {
    pub fn held(&self, button: GamepadButton) -> bool {
        self.held & button.bit() != 0
    }

    pub fn pressed(&self, button: GamepadButton) -> bool {
        self.pressed & button.bit() != 0
    }

    pub fn double_tapped(&self, button: GamepadButton) -> bool {
        self.double_tapped & button.bit() != 0
    }

    /// Whether any stick or trigger is deflected past its dead zone or a button is held.
    pub fn active(&self) -> bool {
        self.left != [0.0; 2]
            || self.right != [0.0; 2]
            || self.triggers != [0.0; 2]
            || self.held != 0
    }

    /// The controller state through the dead zones and curves.
    pub fn from_state(state: &GamepadState, previous_buttons: u32) -> Self {
        Self {
            left: stick(state.sticks[0]),
            right: stick(state.sticks[1]),
            triggers: state.triggers.map(trigger),
            held: state.buttons,
            pressed: state.buttons & !previous_buttons,
            double_tapped: 0,
        }
    }
}

/// A radial dead zone: inside it the stick is centred, beyond it the deflection is rescaled from
/// the dead zone to the saturation edge, curved, and kept in its direction.
pub fn stick([x, y]: [f32; 2]) -> [f32; 2] {
    let magnitude = x.hypot(y);
    if !magnitude.is_finite() || magnitude <= STICK_DEAD_ZONE {
        return [0.0; 2];
    }
    let scaled = ((magnitude - STICK_DEAD_ZONE) / (STICK_SATURATION - STICK_DEAD_ZONE)).min(1.0);
    let response = scaled.powf(STICK_CURVE);
    [x / magnitude * response, y / magnitude * response]
}

/// A trigger's dead zone, rescaled to the full range beyond it.
pub fn trigger(value: f32) -> f32 {
    if !value.is_finite() || value <= TRIGGER_DEAD_ZONE {
        0.0
    } else {
        ((value - TRIGGER_DEAD_ZONE) / (1.0 - TRIGGER_DEAD_ZONE)).min(1.0)
    }
}

/// Frame-rate independent smoothing: each frame the value closes `1 - e^(-dt / time)` of its gap
/// to the target, so it reaches about 63% of a step change after `time` seconds whatever the
/// frame rate.
pub fn approach(value: f32, target: f32, time: f32, dt: f32) -> f32 {
    if time <= 0.0 {
        return target;
    }
    value + (target - value) * (1.0 - (-dt / time).exp())
}

/// One frame of input from every device.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    /// Wall-clock seconds since the last frame, at most [`LONGEST_STEP`].
    pub dt: f32,
    /// Whether the map holds the pointer for looking around.
    pub captured: bool,
    /// Keys held while the map has keyboard focus, by their place on the keyboard: the key a US
    /// layout has there, so W A S D are the same four keys on every layout.
    pub held: BTreeSet<egui::Key>,
    /// Keys pressed this frame, key repeats excluded, by their place.
    pub pressed: Vec<egui::Key>,
    /// Keys whose press this frame followed another within [`DOUBLE_TAP`], by their place.
    pub double_tapped: Vec<egui::Key>,
    /// Shift, Ctrl and Alt held, while the map has keyboard focus.
    pub modifiers: egui::Modifiers,
    /// Pointer motion in physical pixels while captured, `x` right and `y` down.
    pub look: [f32; 2],
    /// The controller, centred when none is connected or the map does not hold it.
    pub pad: Pad,
}

impl Frame {
    pub fn held(&self, key: egui::Key) -> bool {
        self.held.contains(&key)
    }

    pub fn double_tapped(&self, key: egui::Key) -> bool {
        self.double_tapped.contains(&key)
    }

    /// -1, 0 or 1 from a pair of opposing keys.
    pub fn axis(&self, negative: egui::Key, positive: egui::Key) -> f32 {
        f32::from(i8::from(self.held(positive)) - i8::from(self.held(negative)))
    }
}

/// Input state that lasts between frames.
#[derive(Debug, Default)]
pub struct Input {
    captured: bool,
    focused: bool,
    held: BTreeSet<egui::Key>,
    taps: Vec<(egui::Key, Instant)>,
    pad: Option<GamepadState>,
    /// The buttons of the last controller state read, which presses count against.
    pad_buttons: u32,
    /// The handoff this map holds the controller under, if it holds it.
    pad_owner: Option<GamepadOwner>,
    /// Controller buttons pressed once, waiting for a second press.
    pad_taps: Vec<(u32, Instant)>,
    last: Option<Instant>,
}

/// What the window reports for one frame.
pub struct Window<'a> {
    pub events: &'a [egui::Event],
    /// Whether the map's window has keyboard focus.
    pub focused: bool,
    /// The modifier keys the window reports held.
    pub modifiers: egui::Modifiers,
    pub now: Instant,
}

impl Input {
    /// Whether the map holds the pointer.
    pub fn captured(&self) -> bool {
        self.captured
    }

    /// Takes the pointer for looking around; the caller hides and confines the cursor.
    pub fn capture(&mut self) {
        self.captured = true;
    }

    /// Gives the pointer back and forgets every held key.
    pub fn release(&mut self) {
        self.captured = false;
        self.held.clear();
        self.taps.clear();
    }

    /// A controller message from the host, in the order it sent them.
    pub fn gamepad(&mut self, message: GamepadMessage) {
        match message {
            GamepadMessage::Take { owner, state } => {
                // What is held when the map takes the controller is held, not pressed.
                self.pad_owner = Some(owner);
                self.pad = Some(state);
                self.pad_buttons = state.buttons;
                self.pad_taps.clear();
            }
            GamepadMessage::State { owner, state } => {
                if self.pad_owner == Some(owner) {
                    self.pad = Some(state);
                }
            }
            GamepadMessage::Release => {
                self.pad_owner = None;
                self.pad = None;
                self.pad_buttons = 0;
                self.pad_taps.clear();
            }
        }
    }

    /// Restarts the frame clock, as after the map was hidden.
    pub fn restart_clock(&mut self) {
        self.last = None;
    }

    /// Reads one frame. Escape and focus loss release the pointer and clear held input.
    pub fn frame(&mut self, window: Window<'_>) -> Frame {
        let dt = self
            .last
            .map_or(0.0, |last| window.now.duration_since(last).as_secs_f32())
            .clamp(0.0, LONGEST_STEP);
        self.last = Some(window.now);
        let mut frame = Frame {
            dt,
            ..Frame::default()
        };
        let mut lost = self.focused && !window.focused;
        self.focused = window.focused;
        for event in window.events {
            match event {
                egui::Event::WindowFocused(false) => lost = true,
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    ..
                } => {
                    // By place, a key whose meaning changes while it is down still comes up: Shift
                    // released first turns Plus back into Equals.
                    let key = physical_key.unwrap_or(*key);
                    if *pressed {
                        if key == egui::Key::Escape {
                            self.release();
                            continue;
                        }
                        self.held.insert(key);
                        if !repeat {
                            frame.pressed.push(key);
                            let tapped = self.taps.iter().any(|(tapped, at)| {
                                *tapped == key
                                    && window.now.duration_since(*at).as_secs_f32() <= DOUBLE_TAP
                            });
                            self.taps.retain(|(tapped, _)| *tapped != key);
                            if tapped {
                                frame.double_tapped.push(key);
                            } else {
                                self.taps.push((key, window.now));
                            }
                        }
                    } else {
                        self.held.remove(&key);
                    }
                }
                egui::Event::MouseMoved(delta) if self.captured => {
                    frame.look[0] += delta.x;
                    frame.look[1] += delta.y;
                }
                _ => {}
            }
        }
        if lost || !self.focused {
            self.release();
            frame.look = [0.0; 2];
        }
        self.taps
            .retain(|(_, at)| window.now.duration_since(*at).as_secs_f32() <= DOUBLE_TAP);
        frame.captured = self.captured;
        frame.held = self.held.clone();
        if self.focused {
            frame.modifiers = window.modifiers;
        }
        if let Some(state) = &self.pad {
            frame.pad = Pad::from_state(state, self.pad_buttons);
            self.pad_buttons = state.buttons;
        }
        self.pad_taps
            .retain(|(_, at)| window.now.duration_since(*at).as_secs_f32() <= DOUBLE_TAP);
        for button in GamepadButton::ALL {
            let bit = button.bit();
            if frame.pad.pressed & bit == 0 {
                continue;
            }
            if let Some(index) = self.pad_taps.iter().position(|(tapped, _)| *tapped == bit) {
                self.pad_taps.swap_remove(index);
                frame.pad.double_tapped |= bit;
            } else {
                self.pad_taps.push((bit, window.now));
            }
        }
        frame
    }
}

#[cfg(test)]
mod tests;
