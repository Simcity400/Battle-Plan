//! Unidentified embedded camera root used by presentation capture.
//! Owner-vtable authentication and two equal raw reads do not establish hourly
//! consistency. The owner event-list interpretation conflicts with this root's
//! owner pointer for CInGameIdler and with the first view for CNudgeIdler.
//! The concrete root class and complete allocation size remain unknown; these
//! layouts do not establish a safe hourly acquisition.

/// Owner pointer checked against the owner's embedded-root address.
pub const OWNER: usize = 0x08;
/// Starts of the three embedded render views, used to recover the root from a view.
pub const RENDER_VIEWS: [usize; 3] = [0x10, 0x230, 0x450];
/// Camera root read extent, copied twice; the complete allocation size is unknown.
pub const OBSERVED_BYTES: usize = 0x740;
/// Previous-current eye, [f32; 3], preserving three u32 bit patterns.
pub const PREVIOUS_CURRENT_EYE: usize = 0x674;
/// Current eye, [f32; 3], compared with the canonical render view.
pub const CURRENT_EYE: usize = 0x680;
/// Current look-at, [f32; 3], compared with the canonical render view.
pub const CURRENT_LOOK_AT: usize = 0x68c;
/// Target eye, [f32; 3], read before and after event production.
pub const TARGET_EYE: usize = 0x698;
/// Target look-at, [f32; 3], preserving float bits before and after event production.
pub const TARGET_LOOK_AT: usize = 0x6a4;
/// Viewport, [i32; 4].
pub const VIEWPORT: usize = 0x6c0;
/// Minimum height, i32, compared with the maximum.
pub const MIN_HEIGHT: usize = 0x6d0;
/// Maximum height, i32, without float conversion.
pub const MAX_HEIGHT: usize = 0x6d4;
/// Update-produced byte, accepting only zero or one.
pub const UPDATE_PRODUCED: usize = 0x6d8;
/// Special/free-state byte, accepting only zero or one.
pub const SPECIAL_FREE_STATE: usize = 0x6d9;
/// Ramp, f32 preserved as a u32 bit pattern.
pub const RAMP: usize = 0x6f4;
/// Wrap-branch byte, accepting only zero or one.
pub const WRAP_BRANCH: usize = 0x6f8;

/// Camera wheel update receiving the root; this is a presentation boundary.
pub const WHEEL_UPDATE_RVA: usize = 0x0124_e290;
/// Camera wheel update entry sentinel.
pub const WHEEL_UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x4c, 0x89, 0x48, 0x20, 0x55, 0x53, 0x56, 0x48, 0x8d, 0x68, 0x98, 0x48, 0x81,
];
/// Map pick with f32[2] input and f32[3] output; its receiver type is unknown.
pub const MAP_PICK_RVA: usize = 0x00b4_4760;
/// Map-pick entry sentinel.
pub const MAP_PICK_SENTINEL: [u8; 16] = [
    0x4c, 0x8b, 0xdc, 0x45, 0x88, 0x4b, 0x20, 0x49, 0x89, 0x53, 0x10, 0x49, 0x89, 0x4b, 0x08, 0x55,
];
