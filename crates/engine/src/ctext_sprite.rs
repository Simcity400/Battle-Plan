//! Text-sprite presentation layout; no runtime producer.

/// CTextSprite primary vtable.
pub const VTABLE_RVA: usize = 0x02b3_11a0;
/// CTextSprite constructor.
pub const CONSTRUCTOR_RVA: usize = 0x022c_0360;
/// CTextSprite draw virtual slot.
pub const DRAW_VSLOT: usize = 0x188;
/// CTextSprite stored-state draw virtual slot.
pub const DRAW_STORED_VSLOT: usize = 0x190;
/// CTextSprite draw implementation.
pub const DRAW_RVA: usize = 0x022c_1a90;
/// CTextSprite stored-state draw implementation.
pub const DRAW_STORED_RVA: usize = 0x022c_2120;
