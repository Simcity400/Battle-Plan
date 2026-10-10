//! CGameBitmapFont's game-specific inline-icon virtuals. Common font fields are
//! inherited from CBitmapFont. This is preserved presentation knowledge only.

/// CGameBitmapFont vtable.
pub const VTABLE_RVA: usize = 0x029c_ffe8;
/// CGameBitmapFont secondary interface vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x029d_0108;
/// CGameBitmapFont icon lookup context pointer.
pub const ICON_LOOKUP_CONTEXT: usize = 0x3620;
/// CGameBitmapFont lazy icon backing pointer.
pub const ICON_BACKING: usize = 0x3628;
/// CGameBitmapFont lazy icon initialization byte.
pub const ICON_INITIALIZED: usize = 0x3630;
/// Lazy icon backing allocation size; internal layout remains unknown.
pub const ICON_BACKING_ALLOCATION_BYTES: usize = 0x60;
/// CGameBitmapFont icon lookup function.
pub const TEXT_ICON_LOOKUP_RVA: usize = 0x0162_e880;
/// CGameBitmapFont icon drawing function.
pub const TEXT_ICON_DRAW_RVA: usize = 0x0162_efe0;
/// CGameBitmapFont icon advance, native return u32.
pub const TEXT_ICON_ADVANCE_RVA: usize = 0x0162_eb10;
/// CGameBitmapFont icon height function.
pub const TEXT_ICON_HEIGHT_RVA: usize = 0x0162_eac0;
/// CGameBitmapFont icon draw virtual slot.
pub const TEXT_ICON_DRAW_VTABLE_SLOT: usize = 0xe0;
/// CGameBitmapFont icon advance virtual slot.
pub const TEXT_ICON_ADVANCE_VTABLE_SLOT: usize = 0xf0;
/// CGameBitmapFont icon height virtual slot.
pub const TEXT_ICON_HEIGHT_VTABLE_SLOT: usize = 0xf8;
/// CGameBitmapFont icon draw entry bytes.
pub const TEXT_ICON_DRAW_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x55, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x58, 0x80, 0xb9, 0x30, 0x36, 0x00, 0x00,
];
