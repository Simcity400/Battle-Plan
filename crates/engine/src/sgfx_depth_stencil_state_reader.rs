//! Depth/stencil declaration reader for presentation.

/// Reader vtable.
pub const VTABLE_RVA: usize = 0x02b3_b1c8;
/// Declaration-name string following the vtable pointer.
pub const NAME: usize = 0x08;
/// Inline depth/stencil descriptor after the name at +0x28.
pub const DESCRIPTION: usize = 0x28;
