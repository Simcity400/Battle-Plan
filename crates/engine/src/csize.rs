//! Authored GUI dimensions and bounds. Dimensions retain i32 values and
//! byte flags. Double-percentage assignment retains percentage and
//! position-relative bits; later absolute assignments preserve them.
//! Parser/token-stream virtual calls belong to other receivers.

/// Current width, i32.
pub const WIDTH: usize = 0x00;
/// Current height, i32.
pub const HEIGHT: usize = 0x04;
/// Current flags for percentage, position-relative and auto dimensions.
pub const FLAGS: usize = 0x08;
/// Minimum width, i32.
pub const MINIMUM_WIDTH: usize = 0x0c;
/// Minimum height, i32, used only when its auto bit is clear.
pub const MINIMUM_HEIGHT: usize = 0x10;
/// Minimum flags initialized with both auto bits.
pub const MINIMUM_FLAGS: usize = 0x14;
/// Maximum width, i32.
pub const MAXIMUM_WIDTH: usize = 0x18;
/// Maximum height, i32, used only when its auto bit is clear.
pub const MAXIMUM_HEIGHT: usize = 0x1c;
/// Maximum flags initialized with both auto bits.
pub const MAXIMUM_FLAGS: usize = 0x20;
/// Step width, i32, initialized to one.
pub const STEP_WIDTH: usize = 0x24;
/// Step height, i32, initialized to one.
pub const STEP_HEIGHT: usize = 0x28;
/// Step flags initialized to zero.
pub const STEP_FLAGS: usize = 0x2c;
/// Preserve-aspect-ratio boolean byte.
pub const PRESERVE_ASPECT_RATIO: usize = 0x30;
/// Width percentage bit retained on later absolute assignments.
pub const WIDTH_PERCENT: u8 = 0x01;
/// Height percentage bit retained on later absolute assignments.
pub const HEIGHT_PERCENT: u8 = 0x02;
/// Width position-relative bit subtracting x.
pub const WIDTH_RELATIVE_TO_POSITION: u8 = 0x04;
/// Height position-relative bit subtracting y.
pub const HEIGHT_RELATIVE_TO_POSITION: u8 = 0x08;
/// Width auto bit cleared on numeric assignment.
pub const WIDTH_AUTO: u8 = 0x10;
/// Height auto bit cleared on numeric assignment.
pub const HEIGHT_AUTO: u8 = 0x20;
/// Constructor initializing dimensions, bounds flags, unit steps and aspect byte.
pub const CONSTRUCT_RVA: usize = 0x022e_e9f0;
/// Parser selecting dimension pairs and aspect-ratio destination.
pub const PARSE_RVA: usize = 0x022e_f100;
/// Coordinate loader receiving a destination pair with distinct percentage
/// and position-relative branches.
pub const PARSE_COORDINATE_RVA: usize = 0x022f_03a0;
/// Evaluator resolving percentage, negative/relative dimensions, bounds and aspect ratio.
pub const EVALUATE_RVA: usize = 0x016b_9be0;
