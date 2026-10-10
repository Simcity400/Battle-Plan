//! Authored animation records embedded in the sprite-type animation array.
//! These presentation fields do not establish an hourly state walk.

/// Record size and array stride.
pub const SIZE: usize = 0x90;
/// Record vtable.
pub const VTABLE_RVA: usize = 0x02b2_68e8;
/// Deleting destructor destroys both strings.
pub const DELETING_DESTRUCTOR_RVA: usize = 0x0222_1c10;
/// Authored field parser for tokens 490..498 and 502..504.
pub const READ_RVA: usize = 0x0222_c690;
/// Field-parser dispatch offset in the vtable.
pub const READ_VSLOT: usize = 0x20;
/// Move constructor transfers both strings, resets sources and copies remaining fields.
pub const MOVE_CONSTRUCTOR_RVA: usize = 0x0222_1110;
/// Copy constructor copies both strings and remaining fields.
pub const COPY_CONSTRUCTOR_RVA: usize = 0x0222_11f0;
/// Mask filename string at +0x08.
pub const MASK_FILE: usize = 0x08;
/// Texture filename string at +0x28.
pub const TEXTURE_FILE: usize = 0x28;
/// Rotation, f32 radians at +0x48, parsed from degrees.
pub const ROTATION_RADIANS: usize = 0x48;
/// Looping byte at +0x4C, parsed as Boolean.
pub const LOOPING: usize = 0x4c;
/// Duration, f32 at +0x50, initialized to 1.0.
pub const DURATION: usize = 0x50;
/// Delay, f32 at +0x54, initialized to zero.
pub const DELAY: usize = 0x54;
/// Reciprocal texture X scale, f32 at +0x58; zero input supplies 1.0.
pub const RECIPROCAL_TEXTURE_SCALE_X: usize = 0x58;
/// Reciprocal texture Y scale, f32 at +0x5C; zero input supplies 1.0.
pub const RECIPROCAL_TEXTURE_SCALE_Y: usize = 0x5c;
/// Rotation-offset X, f32 at +0x60.
pub const ROTATION_OFFSET_X: usize = 0x60;
/// Rotation-offset Y, f32 at +0x64.
pub const ROTATION_OFFSET_Y: usize = 0x64;
/// Animation kind, 32 bits at +0x68; parsed values 1..4, with zero-kind records discarded.
pub const KIND: usize = 0x68;
/// Blend mode, 32 bits at +0x6C, with values 0..3.
pub const BLEND_MODE: usize = 0x6c;
/// Visible-frame mask, u32 at +0x70; replaced only for a nonempty list.
pub const SHOW_MASK: usize = 0x70;
/// Trigger-frame mask, u32 at +0x74; parsed frame bits are ORed into its previous value.
pub const TRIGGER_MASK: usize = 0x74;
/// Mask texture resource pointer at +0x78, resolved from filename +0x08.
pub const MASK_TEXTURE_RESOURCE: usize = 0x78;
/// Animation texture resource pointer at +0x80, resolved from filename +0x28.
pub const TEXTURE_RESOURCE: usize = 0x80;
/// Texture metadata pair, two i32 values at +0x88, used as dimension divisors.
/// Individual width/height meanings remain unestablished.
pub const TEXTURE_METADATA: usize = 0x88;
/// Initial zero kind; appended only after parsing supplies a nonzero kind.
pub const KIND_UNSET: i32 = 0;
/// Scrolling animation kind.
pub const KIND_SCROLLING: i32 = 1;
/// Clockwise rotation kind.
pub const KIND_ROTATING_CLOCKWISE: i32 = 2;
/// Counterclockwise rotation kind.
pub const KIND_ROTATING_COUNTERCLOCKWISE: i32 = 3;
/// Pulsing animation kind.
pub const KIND_PULSING: i32 = 4;
/// Additive blend mode.
pub const BLEND_ADD: i32 = 0;
/// Multiplicative blend mode.
pub const BLEND_MULTIPLY: i32 = 1;
/// Overlay blend mode.
pub const BLEND_OVERLAY: i32 = 2;
/// Normal blend mode.
pub const BLEND_NORMAL: i32 = 3;
/// Frame index mask; shift counts use their low five bits before setting u32 bits.
pub const FRAME_INDEX_MASK: u32 = 0x1f;
/// Initial visible-frame mask with all 32 bits set.
pub const DEFAULT_SHOW_MASK: u32 = u32::MAX;
/// Initial trigger-frame mask with no bits set.
pub const DEFAULT_TRIGGER_MASK: u32 = 0;
/// Mask-filename parser token.
pub const TOKEN_MASK_FILE: u32 = 490;
/// Texture-filename parser token.
pub const TOKEN_TEXTURE_FILE: u32 = 491;
/// Rotation parser token.
pub const TOKEN_ROTATION: u32 = 492;
/// Looping parser token.
pub const TOKEN_LOOPING: u32 = 493;
/// Duration parser token.
pub const TOKEN_DURATION: u32 = 494;
/// Delay parser token.
pub const TOKEN_DELAY: u32 = 495;
/// Animation-kind parser token.
pub const TOKEN_KIND: u32 = 496;
/// Blend-mode parser token.
pub const TOKEN_BLEND_MODE: u32 = 497;
/// Reciprocal texture-scale parser token.
pub const TOKEN_TEXTURE_SCALE: u32 = 498;
/// Visible-frame mask parser token.
pub const TOKEN_FRAMES: u32 = 502;
/// Rotation-offset pair parser token.
pub const TOKEN_ROTATION_OFFSET: u32 = 503;
/// Accumulating trigger-frame mask parser token.
pub const TOKEN_TRIGGER_FRAMES: u32 = 504;
/// Degrees-to-radians f32 operand.
pub const DEGREES_TO_RADIANS_RVA: usize = 0x0292_ef50;
