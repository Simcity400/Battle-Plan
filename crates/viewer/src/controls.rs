
pub mod flight;
pub mod orbit;
pub mod walker;

use dashboard_core::protocol::{GamepadButton, GamepadMessage, GamepadOwner, GamepadState};
use eframe::egui;
use std::collections::BTreeSet;
use std::time::Instant;

pub const LONGEST_STEP: f32 = 0.1;

pub const STICK_DEAD_ZONE: f32 = 0.18;
pub const STICK_SATURATION: f32 = 0.95;
pub const STICK_CURVE: f32 = 1.6;
pub const TRIGGER_DEAD_ZONE: f32 = 0.06;
pub const DOUBLE_TAP: f32 = 0.3;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    pub left: [f32; 2],
    pub right: [f32; 2],
    pub triggers: [f32; 2],
    pub held: u32,
    pub pressed: u32,
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

    pub fn active(&self) -> bool {
        self.left != [0.0; 2]
            || self.right != [0.0; 2]
            || self.triggers != [0.0; 2]
            || self.held != 0
    }

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

pub fn stick([x, y]: [f32; 2]) -> [f32; 2] {
    let magnitude = x.hypot(y);
    if !magnitude.is_finite() || magnitude <= STICK_DEAD_ZONE {
        return [0.0; 2];
    }
    let scaled = ((magnitude - STICK_DEAD_ZONE) / (STICK_SATURATION - STICK_DEAD_ZONE)).min(1.0);
    let response = scaled.powf(STICK_CURVE);
    [x / magnitude * response, y / magnitude * response]
}

pub fn trigger(value: f32) -> f32 {
    if !value.is_finite() || value <= TRIGGER_DEAD_ZONE {
        0.0
    } else {
        ((value - TRIGGER_DEAD_ZONE) / (1.0 - TRIGGER_DEAD_ZONE)).min(1.0)
    }
}

pub fn approach(value: f32, target: f32, time: f32, dt: f32) -> f32 {
    if time <= 0.0 {
        return target;
    }
    value + (target - value) * (1.0 - (-dt / time).exp())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Device {
    #[default]
    Keyboard,
    Controller,
}

#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub dt: f32,
    pub captured: bool,
    pub held: BTreeSet<egui::Key>,
    pub pressed: Vec<egui::Key>,
    pub double_tapped: Vec<egui::Key>,
    pub modifiers: egui::Modifiers,
    pub look: [f32; 2],
    pub pad: Pad,
    pub pad_connected: bool,
    pub device: Device,
}

impl Frame {
    pub fn held(&self, key: egui::Key) -> bool {
        self.held.contains(&key)
    }

    pub fn pressed(&self, key: egui::Key) -> bool {
        self.pressed.contains(&key)
    }

    pub fn double_tapped(&self, key: egui::Key) -> bool {
        self.double_tapped.contains(&key)
    }

    pub fn axis(&self, negative: egui::Key, positive: egui::Key) -> f32 {
        f32::from(i8::from(self.held(positive)) - i8::from(self.held(negative)))
    }

    pub fn moved_or_looked(&self) -> bool {
        self.look != [0.0; 2]
            || self.pad.left != [0.0; 2]
            || self.pad.right != [0.0; 2]
            || self.pad.triggers != [0.0; 2]
            || self.modifiers.shift
            || [
                egui::Key::W,
                egui::Key::A,
                egui::Key::S,
                egui::Key::D,
                egui::Key::Space,
                egui::Key::ArrowLeft,
                egui::Key::ArrowRight,
                egui::Key::ArrowUp,
                egui::Key::ArrowDown,
            ]
            .into_iter()
            .any(|key| self.held(key))
    }
}

#[derive(Debug, Default)]
pub struct Input {
    captured: bool,
    focused: bool,
    held: BTreeSet<egui::Key>,
    taps: Vec<(egui::Key, Instant)>,
    pad: Option<GamepadState>,
    pad_buttons: u32,
    pad_pressed: u32,
    pad_suppressed: u32,
    pad_owner: Option<GamepadOwner>,
    pad_taps: Vec<(u32, Instant)>,
    device: Device,
    last: Option<Instant>,
}

pub struct Window<'a> {
    pub events: &'a [egui::Event],
    pub focused: bool,
    pub modifiers: egui::Modifiers,
    pub now: Instant,
}

impl Input {
    pub fn captured(&self) -> bool {
        self.captured
    }

    pub fn capture(&mut self) {
        self.captured = true;
    }

    pub fn release(&mut self) {
        self.captured = false;
        self.held.clear();
        self.taps.clear();
    }

    pub fn forget_pad_taps(&mut self) {
        self.pad_taps.clear();
    }

    pub fn suppress_pad(&mut self, button: GamepadButton) {
        self.pad_suppressed |= button.bit();
    }

    pub fn gamepad(&mut self, message: GamepadMessage) {
        match message {
            GamepadMessage::Take { owner, state } => {
                self.pad_owner = Some(owner);
                self.pad = Some(state);
                self.pad_buttons = state.buttons;
                self.pad_pressed = 0;
                self.pad_suppressed = 0;
                self.pad_taps.clear();
            }
            GamepadMessage::State {
                owner,
                state,
                pressed,
            } => {
                if self.pad_owner == Some(owner) {
                    self.pad = Some(state);
                    self.pad_pressed |= pressed;
                }
            }
            GamepadMessage::Release => {
                self.pad_owner = None;
                self.pad = None;
                self.pad_buttons = 0;
                self.pad_pressed = 0;
                self.pad_suppressed = 0;
                self.pad_taps.clear();
                self.device = Device::Keyboard;
            }
        }
    }

    pub fn restart_clock(&mut self) {
        self.last = None;
    }

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
        let mut kbm_activity = false;
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
                    let key = physical_key.unwrap_or(*key);
                    if *pressed {
                        kbm_activity = true;
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
                    if delta.x != 0.0 || delta.y != 0.0 {
                        kbm_activity = true;
                    }
                    frame.look[0] += delta.x;
                    frame.look[1] += delta.y;
                }
                egui::Event::PointerButton { pressed: true, .. } => {
                    kbm_activity = true;
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
            self.pad_suppressed &= state.buttons;
            frame.pad = Pad::from_state(state, self.pad_buttons);
            frame.pad.pressed |= std::mem::take(&mut self.pad_pressed);
            self.pad_buttons = state.buttons;
            frame.pad.held &= !self.pad_suppressed;
            frame.pad.pressed &= !self.pad_suppressed;
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
        if frame.pad.active() || frame.pad.pressed != 0 {
            self.device = Device::Controller;
        } else if kbm_activity {
            self.device = Device::Keyboard;
        }
        frame.pad_connected = self.pad.is_some();
        frame.device = self.device;
        frame
    }
}

#[cfg(test)]
mod tests;
