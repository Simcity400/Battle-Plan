//! CGameIdler GUI frame clock, inherited by CInGameIdler at displacement zero.
//!
//! Fields use the primary-object base. The time-provider subobject is at +8;
//! the original declaring base of the time fields remains unknown.
//! This clock follows GUI updates, not game-hour increments.

/// CGameIdler primary vtable.
pub const VTABLE_RVA: usize = 0x0272_0260;
/// CGameIdler CGameTimeProvider vtable.
pub const TIME_PROVIDER_VTABLE_RVA: usize = 0x0272_0610;
/// CGameIdler CGameTimeProvider base.
pub const TIME_PROVIDER_BASE: usize = 0x08;
/// GUI clock constructor initializes its time fields to zero.
pub const CONSTRUCTOR_RVA: usize = 0x0029_d3e0;
/// CGameIdler GUI update.
pub const UPDATE_RVA: usize = 0x002a_0290;
/// CGameIdler primary-relative frame delta f32 seconds.
pub const FRAME_DELTA_SECONDS: usize = 0x18;
/// CGameIdler primary-relative previous time f32 seconds.
pub const PREVIOUS_SECONDS: usize = 0x30;
/// GUI delta upper clamp f32 global.
pub const DELTA_UPPER_CLAMP_RVA: usize = 0x0270_9c30;
/// GUI delta upper clamp exact f32 bits.
pub const DELTA_UPPER_CLAMP_BITS: u32 = 0x3dcc_cccd;
