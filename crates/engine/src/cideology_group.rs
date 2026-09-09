//! CIdeologyGroup definition shared by country identity and politics.

/// CIdeologyGroup primary vtable, confirmed by pinned-exe RTTI; old offsets.rs:64.
pub const VTABLE_RVA: usize = 0x0292_6080;
/// CIdeologyGroup token std::string; old cached_ideology_group_token reads offsets.rs:107 after vtable validation.
pub const TOKEN: usize = 0x18;
