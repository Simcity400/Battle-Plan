//! Active-alert manager reached from the UI root; exact C++ class unproven.
//! This is presentation state, not the country's complete alert causes.

/// Alert manager's per-type CPdxArray base; old hud_role_observer_live/src/a729_hook.rs:50,383 walks +16950.
pub const TYPE_VECTORS: usize = 0x16950;
/// Alert manager per-type CPdxArray stride; old a729_hook.rs:51,383 multiplies type by 18.
pub const TYPE_VECTOR_STRIDE: usize = 0x18;
/// Alert manager's engine alert type count; old hud_role_observer_live/src/lib.rs:22 and a729_hook.rs:382 bound the census at 52 hex.
pub const ALERT_TYPE_COUNT: usize = 0x52;
