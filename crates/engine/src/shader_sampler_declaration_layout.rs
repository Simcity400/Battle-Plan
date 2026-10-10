//! Parsed shader sampler declaration for presentation.

/// Declaration row size.
pub const STRIDE: usize = 0x70;
/// Declaration allocation alignment.
pub const ALIGNMENT: usize = 0x10;
/// Declaration-name string at the row start.
pub const NAME: usize = 0;
/// Sampler type, u32, copied from reader +0x30 to declaration +0x20.
pub const TYPE: usize = 0x20;
/// Sampler descriptor at row +0x30, passed to the selected lowering function.
pub const DESCRIPTION: usize = 0x30;
