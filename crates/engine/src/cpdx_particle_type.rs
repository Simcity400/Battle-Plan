//! Particle-type descriptor reached from the particle-object descriptor field.
//! This module enables no recorded table or hook.

/// Render subbucket, u16.
pub const RENDER_SUBBUCKET: usize = 0x38;
/// Particle-type primary vtable.
pub const VTABLE_RVA: usize = 0x02b2_a838;
/// Particle-type allocation size.
pub const SIZE: usize = 0xd8;
/// Synthesized attachment-token string.
pub const ATTACHMENT_TOKEN: usize = 0x10;
/// Authored scalar, f32, passed to the particle runtime.
pub const AUTHORED_SCALE: usize = 0x30;
/// Resolved authored-type pointer.
pub const RESOLVED_TYPE: usize = 0x40;
/// Authored effect-token string.
pub const EFFECT_TOKEN: usize = 0xb8;
/// Particle-type constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0228_1570;
/// Binds the descriptor token and resolved type.
pub const BIND_RVA: usize = 0x0228_1a80;
/// Location of the engine-synthesized attachment-name prefix.
pub const ATTACHMENT_PREFIX_RVA: usize = 0x02ae_6b58;
/// Process-global attachment-name counter; identities are not stable
/// across processes.
pub const ATTACHMENT_COUNTER_RVA: usize = 0x0343_6210;
