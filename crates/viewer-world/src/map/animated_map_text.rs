//! The game's floating map text for the recorded map events: their admission
//! for the viewing country (`events`), the content-defined text types
//! (`content`), the viewer's per-province queues (`player`) and the native
//! rise-and-fade draw (`draw`).
mod content;
mod draw;
pub mod events;
mod player;

pub use content::{Assets, Style};
pub use player::{Player, Popup};
