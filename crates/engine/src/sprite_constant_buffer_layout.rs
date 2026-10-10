//! CPU constant packet for sprite drawing. The ordinary button-state
//! shader allocates 112 bytes, but writes end at byte 104. The eight
//! alignment bytes have no established contents.

/// Sprite constant-buffer handle global.
pub const BUFFER_HANDLE_RVA: usize = 0x0343_a178;
/// World-view-projection matrix, sixteen f32 words at bytes 0..64.
pub const WORLD_VIEW_PROJECTION_MATRIX: usize = 0;
/// Color, four f32 words at +64, with multiplied opacity.
pub const COLOR: usize = 64;
/// Current-frame UV offset, two f32 words at +80.
pub const OFFSET: usize = 80;
/// Next-frame UV offset, two f32 words at +88.
pub const NEXT_OFFSET: usize = 88;
/// Current UI clock, f32 at +96, from draw context +0x32590.
pub const TIME: usize = 96;
/// Retained animation clock, f32 at +100, supplied as argument six.
pub const ANIMATION_TIME: usize = 100;
/// Initialized draw byte extent.
pub const WRITTEN_BYTES: usize = 104;
/// Ordinary button-state reflected packet size including alignment tail.
pub const BUTTON_STATE_BUFFER_BYTES: usize = 112;
/// Static-button reflected packet size; consumes the 96-byte prefix, with only X at +80.
pub const STATIC_BUTTON_BUFFER_BYTES: usize = 96;
