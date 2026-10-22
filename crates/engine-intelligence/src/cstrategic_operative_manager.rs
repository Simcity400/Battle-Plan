//! Strategic-operative manager rooted in the game state when its feature is
//! active. Layout alone does not establish hourly consistency.

/// Manager allocation size.
pub const SIZE: usize = 0x68;
/// Manager constructor installing its vtable.
pub const CONSTRUCTOR_RVA: usize = 0x0e9_db70;
/// Operative pointer-vector data.
pub const OPERATIVES_DATA: usize = 0x08;
/// Pointer-vector element width.
pub const OPERATIVE_STRIDE: usize = 8;
/// Serializer keys each vector entry by its zero-based slot before writing
/// the strategic-operative object.
#[cfg(test)]
pub const SAVE_RVA: usize = 0x0e9_e510;
