//! The visual mods a source's widgets show: one presentation identity per source, shared by
//! every widget artwork consumer, and the presentation stack it mounts.
//!
//! Widgets follow the campaign's resolved visual mod selection, the one its Campaign map shows
//! (the game, global, then campaign; a dashboard map's own widget scope is the map's alone). The
//! service binds each source to its own campaign's identity ([`crate::Source::bind`]),
//! so a selection never leaks from one source to another. Game-look artwork, interface, fonts
//! and native-window text read the presentation stack; recorded facts, simulation and analytics
//! labels keep reading the recorded layers.

pub use dashboard_core::presentation::{
    Admitted, Binding, Cancel, Config, Presentation, Presented, rebind, recorded_mods,
};

#[cfg(test)]
mod tests;
