//! Blend-state declaration reader for presentation.

/// Reader vtable.
pub const VTABLE_RVA: usize = 0x02b3_b178;
/// Declaration-name string following the vtable pointer.
pub const NAME: usize = 0x08;
/// Inline blend descriptor with enable byte at reader +0x28.
pub const DESCRIPTION: usize = 0x28;
/// Field dispatcher writing the inline descriptor from ordered authored tokens.
pub const READ_FIELD_RVA: usize = 0x0234_7e10;
