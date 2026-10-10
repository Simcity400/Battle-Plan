//! Sampler wrapper retained for presentation.

/// Wrapper allocation size for API object and retained descriptor.
pub const SIZE: usize = 0x50;
/// D3D sampler output pointer at the wrapper base.
pub const D3D_SAMPLER: usize = 0;
/// Copied descriptor at wrapper +0x10.
pub const DESCRIPTION: usize = 0x10;
