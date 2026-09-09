//! Anonymous CPU constant packet written by CSpriteType::Draw (FUN14222D5A0).
//! Offsets describe native f32 uploads, not a simulation struct. The ordinary
//! button-state shader allocates 112 bytes but native writes end at byte 104;
//! the eight alignment bytes are not established data and must not be invented.

/// Sprite constant-buffer handle global; the archived FUN14222D5A0 decompile passes DAT14343A178 to upload and bind dispatches.
pub const BUFFER_HANDLE_RVA: usize = 0x0343_a178;
/// Uploaded world-view-projection matrix, sixteen native f32 words; container_sprite_draw.rs:424-446 and :481 confirm byte range 0..64.
pub const WORLD_VIEW_PROJECTION_MATRIX: usize = 0;
/// Uploaded color, four native f32 words; FUN14222D5A0 writes ModColor with multiplied opacity and container_sprite_draw.rs:482 maps it to +64.
pub const COLOR: usize = 64;
/// Current frame's UV offset, two native f32 words; FUN14222D5A0 calls FUN14222A990 and container_sprite_draw.rs:486 maps the result to +80.
pub const OFFSET: usize = 80;
/// Next frame's UV offset, two native f32 words; FUN14222D5A0 repeats FUN14222A990 with frame+1 and container_sprite_draw.rs:488 maps it to +88.
pub const NEXT_OFFSET: usize = 88;
/// Current native UI clock, f32; FUN14222D5A0 copies its context+32590 value and container_sprite_draw.rs:491 maps it to +96.
pub const TIME: usize = 96;
/// Retained sprite animation clock, f32; FUN14222D5A0 copies its sixth argument and container_sprite_draw.rs:492-494 maps it to +100.
pub const ANIMATION_TIME: usize = 100;
/// Number of bytes written by native draw; container_sprite_draw.rs:430 distinguishes the 104 initialized bytes from alignment tail.
pub const WRITTEN_BYTES: usize = 104;
/// Ordinary button-state shader packet size; container_sprite_draw.rs:453-459 confirms reflection expects 112 including eight alignment bytes.
pub const BUTTON_STATE_BUFFER_BYTES: usize = 112;
/// Ordinary static-button shader packet size; container_sprite_draw.rs:455-456 confirms reflection consumes the 96-byte prefix, with only X at +80.
pub const STATIC_BUTTON_BUFFER_BYTES: usize = 96;
