//! Owner of a persistent embedded camera view; its concrete type is unknown.
//! No identity with a gameplay camera root, idler or renderer is assumed.

/// Embedded persistent view subobject.
pub const VIEW: usize = 0x2f30;
/// Update the embedded view eye and target, then its matrices.
pub const UPDATE_VIEW_RVA: usize = 0x0209_8c80;
