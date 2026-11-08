//! Map audio for interactive replay windows: the game's sound effects for the
//! entity events the map presents, mixed off the frame thread.
//!
//! [`engine`] holds the device-free instance and mixer model. [`System`] runs
//! it for one process: a control thread owns the engine and renders ahead into
//! a short queue the output device drains, and a loader thread decodes sample
//! files through the recorded content stack. Frames only send messages, so a
//! slow decode or device never holds a map frame. Exports and dumps never
//! create a [`System`].

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod battle;
pub mod engine;
pub mod music;
mod sdl;
mod system;

pub use battle::Battles;
pub use engine::{Content, Engine, Load, Samples, Started};
pub use music::Chances;
pub use system::{Owner, Source, Status, System};

/// The viewer's volume groups. The game applies its one effects volume to
/// all three; the viewer lets them differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// Unit movement, idle and every other entity sound.
    #[default]
    Units,
    /// Sounds of entities presented in combat.
    Combat,
    /// The interface's button sounds, played at the listener.
    Interface,
}

/// One entity event that names a sound effect, as presented this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Emitter {
    /// Stable while the event stays present; a new key is a new instance.
    pub key: u64,
    pub effect: std::sync::Arc<str>,
    pub position: [f32; 3],
    /// Seconds since the event fired. A falling age is a new firing.
    pub age: f32,
    pub group: Group,
    /// The sound starts again whenever none of it plays, while the emitter
    /// stays present (battle ambience).
    pub restart: bool,
}

/// The camera as the game's audio listener.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    /// The camera's eye.
    pub position: [f32; 3],
    /// The camera's right axis.
    pub right: [f32; 3],
    /// The camera's height, which the height category's volume follows.
    pub altitude: f32,
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            right: [1.0, 0.0, 0.0],
            altitude: 0.0,
        }
    }
}

/// Linear volumes in [0, 1], resolved by the view's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    pub master: f32,
    /// The game's second master factor, `dev_master_volume`.
    pub dev_master: f32,
    /// The game's ambient volume, applied to the height category.
    pub ambient: f32,
    pub units: f32,
    pub combat: f32,
    pub interface: f32,
    /// The game's voice volume, applied to the voice category.
    pub voice: f32,
    /// The game's music volume.
    pub music: f32,
    pub muted: bool,
}

impl Default for Mix {
    /// The game's own defaults when its settings store none.
    fn default() -> Self {
        Self {
            master: 1.0,
            dev_master: 0.5,
            ambient: 0.5,
            units: 1.0,
            combat: 1.0,
            interface: 1.0,
            voice: 1.0,
            music: 0.75,
            muted: false,
        }
    }
}

/// One presented map frame's audio inputs.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub listener: Listener,
    pub emitters: Vec<Emitter>,
    pub mix: Mix,
    /// A presentation discontinuity (seek, step back, branch or perspective
    /// change): sounding voices end and events present now do not count as
    /// newly fired.
    pub reset: bool,
    /// Seconds since the owner's previous frame.
    pub elapsed: f32,
    /// The presented recorded hour, for the start log.
    pub hour: Option<i32>,
    /// The songs' chances at the presented hour; `None` before an hour is built.
    pub music: Option<std::sync::Arc<Chances>>,
}
