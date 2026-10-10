//! CIdea definition uses the complete primary base. Accepting a secondary
//! vtable without adjusting that base does not establish another valid
//! token offset.

/// Idea primary vtable.
pub const VTABLE_RVA: usize = 0x0296_a9b0;
/// Idea secondary vtable at complete-object displacement +0x10.
pub const SECONDARY_VTABLE_RVA: usize = 0x0296_aa00;
/// Idea secondary-subobject displacement from primary.
pub const SECONDARY_BASE: usize = 0x10;
/// CIdea token std::string at primary +0x18.
pub const TOKEN: usize = 0x18;
