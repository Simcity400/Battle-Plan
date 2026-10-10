//! Sampler descriptor distinct from D3D11_SAMPLER_DESC. Raw integer LOD
//! limits become floats during lowering; authored anisotropy is parsed
//! but the lowerer supplies its own value. Presentation metadata only.

/// Descriptor byte size, copied as eight qwords into wrapper +0x10.
pub const SIZE: usize = 0x40;
/// Physical sampler slot, u32, selecting the output array entry.
pub const INDEX: usize = 0;
/// LOD bias, f32 bits, copied unchanged.
pub const MIP_LOD_BIAS: usize = 0x04;
/// Minimum LOD, i32, converted to float during lowering.
pub const MIN_LOD: usize = 0x08;
/// Maximum LOD, i32, converted to float during lowering.
pub const MAX_LOD: usize = 0x0c;
/// Authored maximum anisotropy, i32; unused by lowering.
pub const MAX_ANISOTROPY: usize = 0x10;
/// Magnification filter, u32.
pub const MAG_FILTER: usize = 0x14;
/// Minification filter, u32.
pub const MIN_FILTER: usize = 0x18;
/// Mip filter, u32.
pub const MIP_FILTER: usize = 0x1c;
/// U address mode, u32.
pub const ADDRESS_U: usize = 0x20;
/// V address mode, u32.
pub const ADDRESS_V: usize = 0x24;
/// Border color, four f32 values.
pub const BORDER_COLOR: usize = 0x30;
/// None filter encoding.
pub const FILTER_NONE: u32 = 0;
/// Point filter encoding.
pub const FILTER_POINT: u32 = 1;
/// Linear filter encoding and default.
pub const FILTER_LINEAR: u32 = 2;
/// Anisotropic filter encoding.
pub const FILTER_ANISOTROPIC: u32 = 3;
/// Wrap address encoding and default.
pub const ADDRESS_WRAP: u32 = 0;
/// Mirror address encoding.
pub const ADDRESS_MIRROR: u32 = 1;
/// Clamp address encoding.
pub const ADDRESS_CLAMP: u32 = 2;
/// Border address encoding.
pub const ADDRESS_BORDER: u32 = 3;
/// Mirror-once address encoding.
pub const ADDRESS_MIRROR_ONCE: u32 = 4;
/// Filter token scanner; invalid tokens leave the destination unchanged.
pub const READ_FILTER_RVA: usize = 0x023f_7c40;
/// Address token scanner; invalid tokens leave the destination unchanged.
pub const READ_ADDRESS_RVA: usize = 0x023f_7f20;
/// Sampler lowering into a D3D11 wrapper.
pub const LOWER_D3D11_RVA: usize = 0x023f_d550;
/// Filter lowering selects anisotropic if any operand requests it; otherwise tests min/mag/mip linear filters.
pub const LOWER_FILTER_RVA: usize = 0x0241_10f0;
/// Address lowering handles zero through four, falling back to wrap.
pub const LOWER_ADDRESS_RVA: usize = 0x0241_0ef0;
/// Selected sampler creation function pointer for the D3D11 backend.
pub const CREATE_FUNCTION_POINTER_RVA: usize = 0x030a_78f8;
