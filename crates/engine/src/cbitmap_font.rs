//! Bitmap font with a derived game font. Glyph-site descriptors are excluded
//! from hourly sampling.

/// Bitmap-font vtable.
pub const VTABLE_RVA: usize = 0x02b2_aac8;
/// Allocation size.
pub const SIZE: usize = 0x3620;
/// Lost-device interface subobject.
pub const LOST_DEVICE_INTERFACE: usize = 0x40;
/// Border-aware draw virtual byte offset.
pub const BORDER_DRAW_VTABLE_SLOT: usize = 0x48;
/// Border-aware drawing function shared by both font types.
pub const BORDER_DRAW_RVA: usize = 0x0228_c8d0;
/// Inner drawing function.
pub const DRAW_RVA: usize = 0x0228_b2f0;
/// Font loader requesting the descriptor's texture stem.
pub const LOAD_FONT_RVA: usize = 0x0228_7830;
/// Glyph preparation function.
pub const GLYPH_PREPARATION_RVA: usize = 0x0228_5960;
/// Glyph descriptor lookup.
pub const GLYPH_DESCRIPTOR_LOOKUP_RVA: usize = 0x0228_6590;
/// Grouped vertex submission.
pub const GROUPED_SUBMIT_RVA: usize = 0x0228_60e0;
/// Post-emit observation site.
pub const GLYPH_POST_EMIT_RVA: usize = 0x0228_c585;
/// Post-emit instruction sentinel.
pub const GLYPH_POST_EMIT_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0x4d, 0xd0, 0x48, 0x85, 0xc9, 0x4c, 0x8b, 0x54, 0x24, 0x78, 0x74, 0x10, 0x41, 0x8b,
];
/// Complete instruction span for post-emit relocation.
pub const GLYPH_POST_EMIT_INSTRUCTION_SPAN: usize = 7;
/// Source ordinal, u32 at original RSP plus this offset; compared with RBX.
pub const POST_EMIT_SOURCE_ORDINAL_RSP_OFFSET: usize = 0x6c;
/// Vertex count, u64 at RBP minus this displacement.
pub const POST_EMIT_VERTEX_COUNT_RBP_SUBTRACT: usize = 0x60;
/// Scratch global holding six glyph vertices.
pub const GLYPH_VERTEX_SCRATCH_RVA: usize = 0x0345_3170;
/// Fallback codepoint global, u32.
pub const FALLBACK_CODEPOINT_RVA: usize = 0x030a_5e80;
/// Graphics-context pointer inherited by the game font.
pub const GRAPHICS_CONTEXT: usize = 0x48;
/// Viewport width, i32, used by coordinate normalization.
pub const VIEWPORT_WIDTH: usize = 0x50;
/// Viewport height, i32, used by coordinate normalization.
pub const VIEWPORT_HEIGHT: usize = 0x54;
/// Packed text color, u32.
pub const PACKED_TEXT_COLOR: usize = 0x58;
/// Saved packed text color, u32.
pub const SAVED_PACKED_TEXT_COLOR: usize = 0x5c;
/// Extra glyph attribute, f32, copied to vertices; meaning is unknown.
pub const EXTRA_GLYPH_ATTRIBUTE: usize = 0x60;
/// Current RGBA components, f32[4].
pub const TEXT_COLOR_RGBA: usize = 0x80;
/// Saved RGBA components, f32[4].
pub const SAVED_TEXT_COLOR_RGBA: usize = 0xa0;
/// Separate f32[4] color payload with unknown role.
pub const AUXILIARY_COLOR_RGBA: usize = 0xc0;
/// Family/page array pointer.
pub const FAMILIES: usize = 0xe8;
/// Family/page count, i32.
pub const FAMILY_COUNT: usize = 0xf4;
/// Family-vector allocator pointer.
pub const FAMILY_ALLOCATOR: usize = 0xf8;
/// Active family/page index, i32.
pub const ACTIVE_FAMILY: usize = 0x100;
/// Embedded text-color table.
pub const TEXT_COLORS: usize = 0x110;
/// Text-color entry count from the object layout, independent of capture capacity.
pub const TEXT_COLOR_COUNT: usize = 256;
/// Unclassified byte, initially zero.
pub const UNKNOWN_BYTE_3110: usize = 0x3110;
/// Unclassified f32, initially one.
pub const UNKNOWN_FLOAT_3114: usize = 0x3114;
/// Inline glyph-buffer cache array.
pub const GLYPH_BUFFER_CACHE: usize = 0x3118;
/// Cache slot count from the object layout, independent of capture capacity.
pub const GLYPH_BUFFER_CACHE_COUNT: usize = 40;
