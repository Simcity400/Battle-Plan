//! Host-level INPUT events — a backend-agnostic view of pointer/key input the window
//! host routes to view instances (so a view's `on_input` never sees raw winit). The
//! `main.rs` event loop translates winit events into these.

/// A pointer/key event delivered to a view instance, in SCREEN pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewInput {
    /// Left/right button pressed at (x, y).
    PointerDown {
        x: f32,
        y: f32,
        button: PointerButton,
    },
    /// Pointer moved to (x, y); `drag` true while a button is held.
    PointerMove { x: f32, y: f32, drag: bool },
    /// Button released at (x, y).
    PointerUp {
        x: f32,
        y: f32,
        button: PointerButton,
    },
    /// Wheel scrolled `dy` (lines) with the pointer at (x, y).
    Wheel { x: f32, y: f32, dy: f32 },
    /// A logical key was pressed (a small set the overlays care about).
    Key { key: ViewKey },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKey {
    Escape,
}

impl ViewInput {
    /// The pointer position, if this event carries one.
    pub fn pos(&self) -> Option<(f32, f32)> {
        match *self {
            ViewInput::PointerDown { x, y, .. }
            | ViewInput::PointerMove { x, y, .. }
            | ViewInput::PointerUp { x, y, .. }
            | ViewInput::Wheel { x, y, .. } => Some((x, y)),
            ViewInput::Key { .. } => None,
        }
    }
}
