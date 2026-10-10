//! Progressbar-sprite presentation layout; no hourly walker exists.

/// Allocation size.
pub const SIZE: usize = 0x130;
/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02b3_b408;
/// Constructor installing the vtable and initializing orientation.
pub const CONSTRUCTOR_RVA: usize = 0x0234_a110;
/// Authored-key dispatcher selecting the orientation byte.
pub const KEY_DISPATCH_RVA: usize = 0x0234_af40;
/// Texture/effect and vertex builder.
pub const BUILD_RESOURCES_RVA: usize = 0x0234_a5e0;
/// Owner pointer supplied by the second constructor argument; class unresolved.
pub const OWNER: usize = 0x48;
/// Effect-file string.
pub const EFFECT_FILE: usize = 0x50;
/// First colour object; component layout unresolved.
pub const COLOR: usize = 0x70;
/// Second colour object.
pub const COLOR_TWO: usize = 0x90;
/// First texture-file string used by resource loading.
pub const TEXTURE_FILE_1: usize = 0xb0;
/// Second texture-file string used by resource loading.
pub const TEXTURE_FILE_2: usize = 0xd0;
/// First texture signed handle; a negative value is invalid.
pub const TEXTURE_1_HANDLE: usize = 0xf0;
/// Second texture signed handle; a negative value is invalid.
pub const TEXTURE_2_HANDLE: usize = 0xf4;
/// Token-0x1e8 destination with unresolved field width.
pub const TOKEN_01E8_VALUE: usize = 0xf8;
/// Signed size X converted to horizontal vertex coordinates.
pub const SIZE_X: usize = 0xfc;
/// Signed size Y converted to vertical vertex coordinates.
pub const SIZE_Y: usize = 0x100;
/// 32-bit effect-resource handle.
pub const EFFECT_HANDLE: usize = 0x104;
/// Horizontal boolean byte selecting swapped UV axes.
pub const HORIZONTAL: usize = 0x108;
/// Default horizontal byte initialized to one and retained when the key is omitted.
pub const HORIZONTAL_DEFAULT: u8 = 1;
/// Pointer-sized handle for the four-vertex allocation.
pub const VERTEX_BUFFER: usize = 0x110;
/// Transparency byte written by the two accepted transparency tokens.
pub const ALWAYS_TRANSPARENT: usize = 0x118;
/// Texture U scale, f32: texture width divided by padded width.
pub const TEXTURE_U_SCALE: usize = 0x11c;
/// Texture V scale, f32: texture height divided by padded height.
pub const TEXTURE_V_SCALE: usize = 0x120;
/// Orientation parser token.
pub const HORIZONTAL_TOKEN_ID: u32 = 153;
/// Factory parser token for progressbar sprites.
pub const TYPE_TOKEN_ID: u32 = 274;
