//! Shader vector `vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed`.
//!
//! This is the presentation constant layout, not an identified C++ object.
//! The offset is relative to the reflected vector, not the complete buffer.

/// Snow/mud fade is the native f32 third component; portable_replay/native/render_stream/src/lib.rs:49-50 records its reflected byte offset.
pub const FADE: usize = 8;
