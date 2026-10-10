//! Named shader rasterizer declaration for presentation.
//! The stored name uses MSVC string geometry; descriptor fields are in
//! `rasterizer_state_description_layout`.

/// Named-state row stride.
pub const STRIDE: usize = 0x40;
/// Name string at the row start.
pub const NAME: usize = 0x00;
/// Descriptor after the name.
pub const DESCRIPTION: usize = 0x20;
/// Declaration parser and append routine.
pub const READ_DECLARATION_RVA: usize = 0x0234_8f80;
/// Range-copy helper used when the declaration array grows.
pub const COPY_RANGE_RVA: usize = 0x0234_66a0;
