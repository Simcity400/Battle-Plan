//! Anchor-selector hash bucket.

/// Anchor-selector bucket stride; traversal advances without wrapping.
pub const STRIDE: usize = 0x18;
/// Probe marker, u8, at bucket +4.
pub const MARKER: usize = 0x04;
/// Province selector key, u32, at bucket +8.
pub const KEY: usize = 0x08;
/// Selector value, u64, at bucket +0x10; tested for nonzero.
pub const VALUE: usize = 0x10;
