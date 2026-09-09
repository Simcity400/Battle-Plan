//! CIdea definition. Offsets here use the complete object's primary base.
//! The old reader also accepted the secondary vtable without adjusting the token base;
//! that path is retained as a known uncertainty, not a second valid token offset.

/// CIdea primary vtable, confirmed by pinned-exe RTTI; old offsets.rs:62.
pub const VTABLE_RVA: usize = 0x0296_a9b0;
/// CIdea secondary vtable, confirmed by pinned-exe RTTI with complete-object offset 0x10; old offsets.rs:63.
pub const SECONDARY_VTABLE_RVA: usize = 0x0296_aa00;
/// CIdea secondary-subobject displacement from primary, confirmed by the pinned RTTI complete-object locator.
pub const SECONDARY_BASE: usize = 0x10;
/// CIdea token std::string at primary +0x18; old cached_idea_token and domain47_idea_token read it.
pub const TOKEN: usize = 0x18;
