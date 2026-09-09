//! Animation joint receiving the bind-transform copy in FUN1423997a0.
//! Source: replay_interface/native/map_viewer/src/native_animated_bounds.rs:55-56
//! at archive/pre-rewrite-2026-09-09. Old source confirmation only; C++ type unproven.

/// Twelve bind-transform f32 lanes; replay_interface/native/map_viewer/src/native_animated_bounds.rs:55-56 explicitly places the copy at joint+8.
pub const BIND_TRANSFORM: usize = 0x08;
/// Bind-transform lane count copied by FUN1423997a0; replay_interface/native/map_viewer/src/native_animated_bounds.rs:55-56.
pub const BIND_TRANSFORM_LANES: usize = 12;
/// Bind-transform copy endpoint; replay_interface/native/map_viewer/src/native_animated_bounds.rs:55-56 confirms all twelve lanes are copied.
pub const BIND_TRANSFORM_COPY_RVA: usize = 0x0239_97a0;
