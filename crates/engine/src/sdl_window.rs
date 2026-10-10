//! Window layout for event processing. Current client size is separate from
//! retained graphics physical dimensions.

/// Current width, four-byte integer, compared and written with u32 operands.
pub const WIDTH: usize = 0x28;
/// Current height, four-byte integer, compared and written with u32 operands.
pub const HEIGHT: usize = 0x2c;
/// Event handler storing subtype-5 dimensions before the size-change helper.
pub const HANDLE_WINDOW_EVENT_RVA: usize = 0x0210_ab30;
