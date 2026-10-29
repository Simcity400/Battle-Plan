//! CIdea definition uses the complete primary base. Accepting a secondary
//! vtable without adjusting that base does not establish another valid
//! token offset.

/// Idea primary vtable.
pub const VTABLE_RVA: usize = 0x029f_d968;
/// Idea secondary vtable at `SECONDARY_BASE` in the complete object.
pub const SECONDARY_VTABLE_RVA: usize = 0x029f_d9b8;
/// Idea secondary-subobject displacement from primary.
pub const SECONDARY_BASE: usize = 0x10;
/// CIdea token std::string.
pub const TOKEN: usize = 0x18;
