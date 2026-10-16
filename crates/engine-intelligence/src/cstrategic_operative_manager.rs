//! Strategic-operative manager rooted in the game state when its feature is
//! active. Layout alone does not establish hourly consistency.

/// Manager allocation size.
pub const SIZE: usize = 0x68;
/// Manager constructor installing its vtable.
pub const CONSTRUCTOR_RVA: usize = 0x0e9_db70;
/// Initialization constructing one entry per country slot.
pub const INITIALIZE_RVA: usize = 0x0e9_df70;
/// Country lookup into the pointer vector.
pub const LOOKUP_RVA: usize = 0x0e9_dcf0;
/// Operative pointer-vector data.
pub const OPERATIVES_DATA: usize = 0x08;
/// Pointer-vector signed capacity.
pub const OPERATIVES_CAPACITY: usize = 0x10;
/// Pointer-vector signed length.
pub const OPERATIVES_COUNT: usize = 0x14;
/// Vector allocator pointer.
pub const OPERATIVES_ALLOCATOR: usize = 0x18;
/// Pointer-vector element width.
pub const OPERATIVE_STRIDE: usize = 8;
/// Serializer keys each vector entry by its zero-based slot before writing
/// the strategic-operative object.
pub const SAVE_RVA: usize = 0x0e9_e510;
