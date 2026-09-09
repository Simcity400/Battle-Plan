//! Anonymous native sampler descriptor, distinct from D3D11_SAMPLER_DESC.
//!
//! Archived source: `portable_replay/native/engine_evidence/artifacts/`
//! `func_142348d70`, `func_142348890`, `func_1423fd550`, `func_1423f7c40`,
//! `func_1423f7f20`, `func_1424110f0`, and `func_142410ef0` decompiles.
//! Fields retain the engine's raw integers. LOD limits become float only during
//! lowering; MaxAnisotropy is parsed but that lowerer supplies its own value.
//! Presentation knowledge only; native state acquisition is excluded from v1.

/// Descriptor byte size; FUN1423fd550 copies eight qwords into its native wrapper at +0x10.
pub const SIZE: usize = 0x40;
/// Physical sampler slot u32; FUN142345450 reads descriptor+0 and indexes the output array with it.
pub const INDEX: usize = 0;
/// MipMapLodBias f32; FUN142348890 token 0x212 reads reader+0x44 and FUN1423fd550 copies descriptor+4 bits.
pub const MIP_LOD_BIAS: usize = 0x04;
/// MipMapMinLod i32; FUN142348890 reads reader+0x48 and FUN1423fd550 converts descriptor+8 to float.
pub const MIN_LOD: usize = 0x08;
/// MipMapMaxLod i32; FUN142348890 reads reader+0x4c and FUN1423fd550 converts descriptor+0xc to float.
pub const MAX_LOD: usize = 0x0c;
/// Authored MaxAnisotropy i32; FUN142348890 token 0x29b reads reader+0x50; FUN1423fd550 does not consume it.
pub const MAX_ANISOTROPY: usize = 0x10;
/// Native magnification filter u32; FUN142348890 scans reader+0x54 and FUN1423fd550 uses descriptor+0x14.
pub const MAG_FILTER: usize = 0x14;
/// Native minification filter u32; FUN142348890 scans reader+0x58 and FUN1423fd550 uses descriptor+0x18.
pub const MIN_FILTER: usize = 0x18;
/// Native mip filter u32; FUN142348890 scans reader+0x5c and FUN1423fd550 uses descriptor+0x1c.
pub const MIP_FILTER: usize = 0x1c;
/// Native U address mode u32; FUN142348890 scans reader+0x60 and FUN1423fd550 translates descriptor+0x20.
pub const ADDRESS_U: usize = 0x20;
/// Native V address mode u32; FUN142348890 scans reader+0x64 and FUN1423fd550 translates descriptor+0x24.
pub const ADDRESS_V: usize = 0x24;
/// Four f32 border color values; FUN1423fd550 copies descriptor+0x30 and +0x38 into the API descriptor.
pub const BORDER_COLOR: usize = 0x30;
/// Native none filter encoding; FUN1423f7c40 maps the none token at 14271c2a4 to zero.
pub const FILTER_NONE: u32 = 0;
/// Native point filter encoding; FUN1423f7c40 assigns one for point.
pub const FILTER_POINT: u32 = 1;
/// Native linear filter/default encoding; FUN1423f7c40 assigns two and FUN142348d70 initializes all filters to it.
pub const FILTER_LINEAR: u32 = 2;
/// Native anisotropic filter encoding; FUN1423f7c40 assigns three; FUN1424110f0 tests each filter for it.
pub const FILTER_ANISOTROPIC: u32 = 3;
/// Native wrap/default address encoding; FUN1423f7f20 maps the wrap token at 142b1c9f4 to zero.
pub const ADDRESS_WRAP: u32 = 0;
/// Native mirror address encoding; FUN1423f7f20 assigns one for mirror.
pub const ADDRESS_MIRROR: u32 = 1;
/// Native clamp address encoding; FUN1423f7f20 assigns two for clamp.
pub const ADDRESS_CLAMP: u32 = 2;
/// Native border address encoding; FUN1423f7f20 assigns three for border.
pub const ADDRESS_BORDER: u32 = 3;
/// Native mirror-once address encoding; FUN1423f7f20 assigns four for mirror_once.
pub const ADDRESS_MIRROR_ONCE: u32 = 4;
/// Native filter token scanner; archived FUN1423f7c40 leaves the destination unchanged on an invalid token.
pub const READ_FILTER_RVA: usize = 0x023f_7c40;
/// Native address token scanner; archived FUN1423f7f20 leaves the destination unchanged on an invalid token.
pub const READ_ADDRESS_RVA: usize = 0x023f_7f20;
/// Native sampler to D3D11 wrapper lowerer; FUN1423ef860 installs FUN1423fd550 for this backend.
pub const LOWER_D3D11_RVA: usize = 0x023f_d550;
/// Native filter lowering; FUN1424110f0 uses any anisotropic operand, otherwise min/mag/mip linear predicates.
pub const LOWER_FILTER_RVA: usize = 0x0241_10f0;
/// Native address lowering; FUN142410ef0 handles zero through four and falls back to wrap.
pub const LOWER_ADDRESS_RVA: usize = 0x0241_0ef0;
/// Selected sampler creation function pointer; FUN1423ef860 assigns FUN1423fd550 to 1430a78f8 for D3D11.
pub const CREATE_FUNCTION_POINTER_RVA: usize = 0x030a_78f8;
