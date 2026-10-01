//! Anonymous D3D11 renderer created by FUN1423FF310, distinct from CGraphics.
//! Its lowerers receive this object; render_context_layout is a separate binding
//! cache whose +280 pointer reaches the renderer's default-state fields.
//! These facts come from retained a729 fixed-state exports, without new live proof.

/// Native D3D11 renderer creation; archived func_1423ff310.decompile.c writes the backend's device, AA request and default state objects.
pub const CREATE_RVA: usize = 0x023f_f310;
/// Creation-time AA request raw 32-bit value; FUN1423FF310:221 copies creation arguments+18, and FUN1423FD2F0 tests renderer+54 for nonzero.
pub const CREATION_AA_REQUEST: usize = 0x54;
/// Default blend wrapper pointer; FUN1423FF310:277-278 stores the blend lowerer's result at +58, read through context+280 by FUN142400090.
pub const DEFAULT_BLEND_STATE: usize = 0x58;
/// Default depth/stencil wrapper pointer; FUN1423FF310:288-289 stores the depth lowerer's result at +60.
pub const DEFAULT_DEPTH_STENCIL_STATE: usize = 0x60;
/// Default rasterizer wrapper pointer; FUN1423FF310:296-297 stores the raster lowerer's result at +68, read through context+280 by FUN142400330.
pub const DEFAULT_RASTERIZER_STATE: usize = 0x68;
/// D3D device pointer; FUN1423FF310 uses `puVar6[14]` for device calls and FUN1423FD2F0/FUN1423FD550 receive that same backend for state lowering.
pub const DEVICE: usize = 0x70;
