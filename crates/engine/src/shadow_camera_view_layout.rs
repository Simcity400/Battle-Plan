//! Map-scene shadow camera. Aliases with camera root and carrier are
//! unestablished. Presentation metadata only.

/// Focus X scale, f32; camera-root layout instead treats +8 as an owner pointer.
pub const FOCUS_X_SCALE: usize = 0x08;
/// Focus ratio denominator, f32.
pub const FOCUS_RATIO_DENOMINATOR: usize = 0x18;
/// Focus Z scale, f32.
pub const FOCUS_Z_SCALE: usize = 0x28;
/// Focus X offset, f32.
pub const FOCUS_X_OFFSET: usize = 0x180;
/// Focus ratio numerator, f32.
pub const FOCUS_RATIO_NUMERATOR: usize = 0x184;
/// Focus Z offset, f32.
pub const FOCUS_Z_OFFSET: usize = 0x188;
/// Camera Y, f32.
pub const CAMERA_Y: usize = 0x194;
/// West wrap gate byte.
pub const WRAP_WEST_ENABLED: usize = 0x670;
/// East wrap gate byte.
pub const WRAP_EAST_ENABLED: usize = 0x671;
/// Overall wrap gate byte.
pub const WRAP_ENABLED: usize = 0x6f8;
