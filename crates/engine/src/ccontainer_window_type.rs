//! Authored container-window definition. This presentation layout is excluded
//! from hourly recording.

/// Definition vtable.
pub const VTABLE_RVA: usize = 0x02b3_6ea0;
/// Allocation size.
pub const SIZE: usize = 0x598;
/// Constructor.
pub const CONSTRUCTOR_RVA: usize = 0x022f_99b0;
/// Field parser.
pub const PARSE_FIELD_RVA: usize = 0x022f_af80;
/// Background materializer appending created widgets to its destination array.
pub const MATERIALIZE_BACKGROUNDS_RVA: usize = 0x022f_a2e0;
/// Background-definition pointer array.
pub const BACKGROUNDS: usize = 0x270;
/// Background capacity, i32.
pub const BACKGROUNDS_CAPACITY: usize = 0x278;
/// Background count, i32.
pub const BACKGROUNDS_COUNT: usize = 0x27c;
/// Base position copied to both transition endpoints.
pub const POSITION: usize = 0x360;
/// Shown position copied to runtime.
pub const SHOW_POSITION: usize = 0x390;
/// Hidden position copied to runtime.
pub const HIDE_POSITION: usize = 0x3c0;
/// Authored size record.
pub const AUTHORED_SIZE: usize = 0x4e4;
/// Margin record, copied separately from position.
pub const MARGIN: usize = 0x518;
/// Secondary margin record.
pub const SECONDARY_MARGIN: usize = 0x530;
/// Show duration, i32 milliseconds, converted to runtime seconds.
pub const SHOW_ANIMATION_MILLISECONDS: usize = 0x558;
/// Hide duration, i32 milliseconds.
pub const HIDE_ANIMATION_MILLISECONDS: usize = 0x55c;
/// Fade duration, i32 milliseconds, converted to runtime seconds.
pub const FADE_MILLISECONDS: usize = 0x560;
/// Fade delay, i32 milliseconds, converted to runtime seconds.
pub const FADE_DELAY_MILLISECONDS: usize = 0x564;
/// Fade easing enum, 32 bits, copied unchanged.
pub const FADE_EASING: usize = 0x568;
/// Show easing enum, 32 bits.
pub const SHOW_ANIMATION_EASING: usize = 0x57c;
/// Hide easing enum, 32 bits.
pub const HIDE_ANIMATION_EASING: usize = 0x580;
/// Orientation enum, 32 bits.
pub const ORIENTATION: usize = 0x584;
/// Origin enum, 32 bits.
pub const ORIGO: usize = 0x588;
/// Unclassified behavior byte, initialized within a word set to one.
pub const BEHAVIOR_BYTE_58C: usize = 0x58c;
/// Moveable boolean byte.
pub const MOVEABLE: usize = 0x58e;
/// Clipping tri-state, i8, initially -1; parsing assigns zero or one.
pub const CLIPPING: usize = 0x592;
/// Unspecified clipping sentinel; construction derives it from materialized backgrounds.
pub const CLIPPING_UNSPECIFIED: i8 = -1;
