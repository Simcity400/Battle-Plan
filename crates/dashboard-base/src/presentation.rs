//! The visual mods a source's widgets show: one presentation identity per source, shared by
//! every widget artwork consumer, and the presentation stack it mounts.
//!
//! Widgets follow the campaign's resolved visual mod selection, the one its Campaign map shows
//! (the game, global, then campaign; a dashboard map's own widget scope is the map's alone). The
//! service binds each source to its own campaign's identity ([`crate::Source::bind`]),
//! so a selection never leaks from one source to another. Art (textures, fonts, shaders) reads
//! the presentation stack; recorded facts (interface declarations and text among them),
//! simulation and analytics labels are the recording's, which a visual mod never replaces.

pub use dashboard_core::presentation::{
    Admitted, Binding, Cancel, Config, Presentation, Presented, rebind, recorded_mods,
};

#[cfg(test)]
mod tests;
