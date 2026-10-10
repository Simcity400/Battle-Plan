//! Sprite presentation instance. These constants supply no hourly state
//! walker. Inherited position, matrix and clip are in `cgraphical_object`.

/// Constructor initializing crop, texture scale and flip.
pub const CONSTRUCTOR_RVA: usize = 0x022e_9210;
/// Sprite draw implementation.
pub const DRAW_RVA: usize = 0x022e_95c0;
/// Draw virtual-dispatch byte offset.
pub const DRAW_VSLOT: usize = 0x50;
/// Retained-clock query.
pub const CLOCK_GETTER_RVA: usize = 0x022e_95a0;
/// Retained clock-query dispatch byte offset, supplying draw argument five.
pub const CLOCK_GETTER_VSLOT: usize = 0x88;
/// Effect-setter dispatch byte offset used when transferring a signed effect
/// to replacement sprites.
pub const EFFECT_SETTER_VSLOT: usize = 0x98;
/// Integer-size query forwarding to sprite-type size.
pub const SIZE_GETTER_RVA: usize = 0x022e_9530;
/// Integer-size query dispatch byte offset.
pub const SIZE_GETTER_VSLOT: usize = 0x158;
/// Texture V scale, f32, copied to its type.
pub const TEXTURE_V_SCALE: usize = 0x174;
/// U-flip flag copied to type +0x110; storage width unresolved.
pub const FLIP_U: usize = 0x178;
/// Opacity, f32. The setter stores instance +0x18c; drawing copies it
/// to type +0x1a4.
pub const OPACITY: usize = 0x18c;
/// Right crop, u32, copied to its type.
pub const CROP_RIGHT: usize = 0x190;
/// Bottom crop, u32, copied to its type.
pub const CROP_BOTTOM: usize = 0x194;
/// Left crop, u32, copied to its type.
pub const CROP_LEFT: usize = 0x198;
/// Top crop, u32, copied to its type.
pub const CROP_TOP: usize = 0x19c;

/// Generic sprite-render entry sentinel bytes.
pub const DRAW_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x68, 0x18, 0x56, 0x57, 0x41, 0x56, 0x48, 0x81, 0xec, 0xb0, 0x00,
];
