//! Post-effect owner shared by target creation and adaptation.

/// Post-effect constructor initializing targets and flags.
pub const CONSTRUCT_RVA: usize = 0x00ff_1340;
/// Per-frame adaptation driver reading and clearing the reset flag.
pub const UPDATE_RVA: usize = 0x00ff_3110;
/// Target-recreation guard selecting allocation.
pub const ENSURE_TARGETS_RVA: usize = 0x00ff_3250;
/// First target pointer.
pub const TARGETS: usize = 0x08;
/// One-shot adaptation reset byte.
pub const RESET_ADAPTATION: usize = 0x40;
/// Retained HDR-mode byte.
pub const HDR_MODE: usize = 0x41;
/// Global HDR mode compared with and copied into the owner.
pub const HDR_MODE_GLOBAL_RVA: usize = 0x0331_7238;
/// HDR adaptation-speed f32 global scaled by frame seconds.
pub const HDR_ADJUSTMENT_SPEED_RVA: usize = 0x0331_e570;
/// Reset adaptation-factor f32 global.
pub const RESET_TAU_DELTA_TIME_RVA: usize = 0x0270_e564;
