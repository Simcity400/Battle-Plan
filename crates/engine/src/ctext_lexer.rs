//! Raw text-token reader, separate from the global token registry.
//! Storage belongs to the lexer base; this module adds no parser or live reader.

/// Text-token reader vtable.
pub const VTABLE_RVA: usize = 0x2b7a560;
/// Text-token reader constructor.
pub const CONSTRUCTOR_RVA: usize = 0x24a63a0;
/// Token reader; a bare token at end of input fails.
pub const READ_TOKEN_RVA: usize = 0x24a7a90;
/// Token-reader vtable byte offset.
pub const READ_TOKEN_VTABLE_SLOT: usize = 0x08;
/// Token-reader entry bytes.
pub const READ_TOKEN_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x10, 0x48, 0x89, 0x70, 0x18, 0x48, 0x89, 0x78, 0x20, 0x55,
];
