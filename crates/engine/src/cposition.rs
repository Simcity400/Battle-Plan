//! Authored GUI coordinates and bounds. Pairs hold two i32 values and a
//! flags byte; absolute assignments retain percentage bits. Percentage
//! evaluation multiplies in signed 32-bit before converting to f32 and
//! rounding. Parser/token-stream virtual calls belong to other receivers.

/// Current x, i32.
pub const X: usize = 0x00;
/// Current y, i32.
pub const Y: usize = 0x04;
/// Current flags byte with percentage and auto bits.
pub const FLAGS: usize = 0x08;
/// Minimum x, i32.
pub const MINIMUM_X: usize = 0x0c;
/// Minimum y, i32, read when the pair auto bit is clear.
pub const MINIMUM_Y: usize = 0x10;
/// Minimum flags byte initialized with both auto bits.
pub const MINIMUM_FLAGS: usize = 0x14;
/// Maximum x, i32.
pub const MAXIMUM_X: usize = 0x18;
/// Maximum y, i32, read when the pair auto bit is clear.
pub const MAXIMUM_Y: usize = 0x1c;
/// Maximum flags byte initialized with both auto bits.
pub const MAXIMUM_FLAGS: usize = 0x20;
/// Step x, i32, initialized to one.
pub const STEP_X: usize = 0x24;
/// Step y, i32, initialized to one.
pub const STEP_Y: usize = 0x28;
/// Step flags byte, initialized to zero.
pub const STEP_FLAGS: usize = 0x2c;
/// X percentage bit set for percentage assignment and retained on absolute writes.
pub const X_PERCENT: u8 = 0x01;
/// Y percentage bit set for percentage assignment and retained on absolute writes.
pub const Y_PERCENT: u8 = 0x02;
/// X auto bit set on auto assignment and cleared on numeric assignment.
pub const X_AUTO: u8 = 0x10;
/// Y auto bit set on auto assignment and cleared on numeric assignment.
pub const Y_AUTO: u8 = 0x20;
/// Constructor initializing coordinates, bounds flags and unit steps.
pub const CONSTRUCT_RVA: usize = 0x022e_e9c0;
/// Parser selecting current, minimum, maximum and step pair loaders.
pub const PARSE_RVA: usize = 0x022e_ed30;
/// Coordinate loader: second argument is the destination pair, not the parser receiver.
pub const PARSE_COORDINATE_RVA: usize = 0x022e_fa70;
/// X evaluator using i32 coordinates, percentage flags and min/max clamps.
pub const EVALUATE_X_RVA: usize = 0x016b_9ad0;
/// Y evaluator using i32 coordinates, percentage flags and min/max clamps.
pub const EVALUATE_Y_RVA: usize = 0x016b_a060;
/// Pixel-coordinate setter updating x/y while retaining flags and bounds.
pub const SET_PIXELS_RVA: usize = 0x0229_e770;
