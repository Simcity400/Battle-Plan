
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod battle;
pub mod engine;
pub mod music;
pub mod offline;
mod sdl;
mod system;

pub use battle::Battles;
pub use engine::{Content, Engine, Load, Samples, Started};
pub use music::Chances;
pub use offline::OfflineMixer;
pub use system::{Owner, Source, Status, System};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    #[default]
    Units,
    Combat,
    Interface,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Emitter {
    pub key: u64,
    pub effect: std::sync::Arc<str>,
    pub position: [f32; 3],
    pub age: f32,
    pub group: Group,
    pub restart: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub position: [f32; 3],
    pub right: [f32; 3],
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    pub master: f32,
    pub dev_master: f32,
    pub ambient: f32,
    pub units: f32,
    pub combat: f32,
    pub interface: f32,
    pub voice: f32,
    pub music: f32,
    pub muted: bool,
}

impl Default for Mix {
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

#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub listener: Listener,
    pub emitters: Vec<Emitter>,
    pub mix: Mix,
    pub reset: bool,
    pub elapsed: f32,
    pub hour: Option<i32>,
    pub music: Option<std::sync::Arc<Chances>>,
}
