//! Snow/mud presentation staging owner. The buffer holds rendered
//! B8G8R8A8_UNORM cells. Its parallel producer does not establish
//! consistency at the hourly simulation boundary or authorize a hook.

/// Staging-owner pointer global.
pub const SINGLETON_RVA: usize = 0x0343_94f0;
/// Staging-owner getter.
pub const GETTER_RVA: usize = 0x0221_6e70;
/// CPU grid pointer at owner +0x348.
pub const GRID: usize = 0x348;
/// Cell size for B8G8R8A8_UNORM output.
pub const CELL_STRIDE: usize = 4;
/// Quarter-resolution grid producer.
pub const PRODUCER_RVA: usize = 0x0124_b710;
/// Exact producer-entry sentinel.
pub const PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
/// Upload call instruction site.
pub const UPLOAD_CALLSITE_RVA: usize = 0x0124_baa4;
/// Shared texture uploader also used by occupation textures.
pub const UPLOAD_TARGET_RVA: usize = 0x0240_0e40;
/// Exact uploader-entry sentinel.
pub const UPLOAD_TARGET_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x60, 0x48,
];
